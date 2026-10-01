// SPDX-License-Identifier: GPL-3.0-or-later
// Headless qualification of the custom ABI, submission capacity and VT import.
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#include <mpv/client.h>
#include <mpv/render.h>
#include <mpv/render_mtl.h>
#include <stdatomic.h>
#include <stddef.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <time.h>

_Static_assert(OXPLAY_NATIVE_RENDER_ABI == 1, "wrong native ABI");
_Static_assert(sizeof(mpv_metal_init_params) == 5 * sizeof(void *), "init layout");
_Static_assert(offsetof(mpv_metal_texture, width) == sizeof(void *), "target layout");
_Static_assert(sizeof(mpv_metal_texture) == 16, "target size");
_Static_assert(MPV_RENDER_PARAM_METAL_INIT_PARAMS == 23, "init ID");
_Static_assert(MPV_RENDER_PARAM_METAL_TEXTURE == 24, "target ID");
_Static_assert(MPV_RENDER_PARAM_METAL_BUSY == 27, "capacity ID");

static atomic_uint wakes;
static void wake(void *context)
{
    (void)context;
    atomic_fetch_add_explicit(&wakes, 1, memory_order_relaxed);
}
static void tick(void)
{
    nanosleep(&(struct timespec){.tv_nsec = 1000000}, NULL);
}
static double seconds(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}
static bool check(int result, const char *operation)
{
    if (result >= 0)
        return true;
    fprintf(stderr, "%s: engine error %d\n", operation, result);
    return false;
}

static uint64_t pixels_hash(id<MTLDevice> device, id<MTLCommandQueue> queue,
                            id<MTLTexture> texture)
{
    const NSUInteger pitch = 320 * 4;
    id<MTLBuffer> readback = [device newBufferWithLength:pitch * 180 options:MTLResourceStorageModeShared];
    id<MTLCommandBuffer> copy = [queue commandBuffer];
    id<MTLBlitCommandEncoder> blit = [copy blitCommandEncoder];
    [blit copyFromTexture:texture sourceSlice:0 sourceLevel:0 sourceOrigin:MTLOriginMake(0, 0, 0)
              sourceSize:MTLSizeMake(320, 180, 1) toBuffer:readback destinationOffset:0
   destinationBytesPerRow:pitch destinationBytesPerImage:pitch * 180];
    [blit endEncoding];
    [copy commit];
    [copy waitUntilCompleted]; // Explicit diagnostic only, never presenter behavior.
    if (copy.status != MTLCommandBufferStatusCompleted)
        return 0;
    uint64_t hash = UINT64_C(14695981039346656037);
    bool colored = false;
    const unsigned char *pixels = readback.contents;
    for (NSUInteger i = 0; i < pitch * 180; i += 4) {
        for (int channel = 0; channel < 3; channel++) {
            colored |= pixels[i + channel] != 0;
            hash = (hash ^ pixels[i + channel]) * UINT64_C(1099511628211);
        }
    }
    return colored ? hash : 0;
}

#ifndef OXPLAY_METAL_SMOKE_HELPERS_ONLY
int main(int argc, const char *argv[])
{
    if (argc != 2 || argv[1][0] != '/') {
        fprintf(stderr, "Usage: native-metal-smoke /absolute/local/h264-fixture.mp4\n");
        return 2;
    }
    @autoreleasepool {
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        id<MTLCommandQueue> queue = [device newCommandQueue];
        if (!device || !queue)
            return 3;
        MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:
            MTLPixelFormatRGBA8Unorm width:320 height:180 mipmapped:NO];
        desc.usage = MTLTextureUsageShaderRead | MTLTextureUsageRenderTarget;
        desc.storageMode = MTLStorageModePrivate;
        id<MTLTexture> texture = [device newTextureWithDescriptor:desc];
        mpv_handle *mpv = mpv_create();
        if (!texture || !mpv)
            return 3;
        mpv_set_option_string(mpv, "config", "no");
        mpv_set_option_string(mpv, "terminal", "no");
        mpv_set_option_string(mpv, "msg-level", "all=no");
        mpv_set_option_string(mpv, "vo", "libmpv");
        mpv_set_option_string(mpv, "hwdec", "videotoolbox");
        mpv_set_option_string(mpv, "ao", "null");
        mpv_set_option_string(mpv, "osc", "no");
        mpv_set_option_string(mpv, "keep-open", "yes");
        if (!check(mpv_initialize(mpv), "initialize"))
            return 4;
        mpv_metal_init_params init = {.layer = NULL,
            .metal_device = (__bridge void *)device, .command_queue = (__bridge void *)queue,
            .wakeup = wake, .wakeup_context = NULL};
        mpv_render_param create[] = {{MPV_RENDER_PARAM_API_TYPE, "metal"},
            {MPV_RENDER_PARAM_METAL_INIT_PARAMS, &init}, {0}};
        mpv_render_context *context = NULL;
        if (!check(mpv_render_context_create(&context, mpv, create), "create Metal context"))
            return 5;
        int block = 0, busy = 0, flip = 0;
        mpv_metal_texture target = {.texture = (__bridge void *)texture, .width = 320, .height = 180};
        mpv_render_param render[] = {{MPV_RENDER_PARAM_METAL_TEXTURE, &target},
            {MPV_RENDER_PARAM_METAL_BUSY, &busy}, {MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME, &block},
            {MPV_RENDER_PARAM_FLIP_Y, &flip}, {0}};
        // A queue wait deterministically fills native slots without heavy work.
        id<MTLSharedEvent> gate = [device newSharedEvent];
        id<MTLCommandBuffer> barrier = [queue commandBuffer];
        [barrier encodeWaitForEvent:gate value:1];
        [barrier commit];
        for (int i = 0; i < 3; i++) {
            if (!check(mpv_render_context_render(context, render), "capacity fill") || busy)
                return 6;
        }
        if (!check(mpv_render_context_get_info(context, (mpv_render_param){MPV_RENDER_PARAM_METAL_BUSY, &busy}),
                   "capacity query") || busy != 1 || atomic_load(&wakes) != 0)
            return 6;
        if (!check(mpv_render_context_render(context, render), "capacity defer") || busy != 1)
            return 6;
        gate.signaledValue = 1;
        double deadline = seconds() + 5;
        while (atomic_load(&wakes) == 0 && seconds() < deadline)
            tick();
        if (atomic_load(&wakes) != 1)
            return 7;
        // Drain native and host work before the hardware-import fixture begins.
        id<MTLCommandBuffer> drain = [queue commandBuffer];
        [drain commit];
        [drain waitUntilCompleted];
        unsigned capacity_wakes = atomic_load(&wakes);
        mpv_observe_property(mpv, 1, "hwdec-current", MPV_FORMAT_STRING);
        const char *load[] = {"loadfile", argv[1], "replace", "-1", NULL};
        if (!check(mpv_command(mpv, load), "load fixture"))
            return 8;
        bool videotoolbox = false;
        unsigned frames = 0, distinct_frames = 0;
        uint64_t hashes[15] = {0};
        deadline = seconds() + 10;
        while (seconds() < deadline && frames < 15) {
            mpv_event *event;
            while ((event = mpv_wait_event(mpv, 0))->event_id != MPV_EVENT_NONE) {
                if (event->event_id == MPV_EVENT_PROPERTY_CHANGE) {
                    mpv_event_property *property = event->data;
                    if (property->format == MPV_FORMAT_STRING && property->data &&
                        !strcmp(property->name, "hwdec-current"))
                        videotoolbox = !strcmp(*(char **)property->data, "videotoolbox");
                }
                if (event->event_id == MPV_EVENT_END_FILE &&
                    ((mpv_event_end_file *)event->data)->error < 0)
                    return 8;
            }
            if (mpv_render_context_update(context) & MPV_RENDER_UPDATE_FRAME) {
                if (!check(mpv_render_context_render(context, render), "render VideoToolbox frame"))
                    return 9;
                if (!busy) {
                    uint64_t hash = pixels_hash(device, queue, texture);
                    bool unique = hash != 0;
                    for (unsigned i = 0; i < frames; i++)
                        unique &= hashes[i] != hash;
                    distinct_frames += unique;
                    hashes[frames++] = hash;
                }
            }
            tick();
        }
        mpv_render_context_free(context);
        mpv_terminate_destroy(mpv);
        unsigned final_wakes = atomic_load(&wakes);
        tick();
        if (!videotoolbox || frames < 15 || distinct_frames < 10 || atomic_load(&wakes) != final_wakes) {
            fprintf(stderr, "native content check failed: frames=%u distinct_frames=%u hardware=%d\n",
                    frames, distinct_frames, videotoolbox);
            return 10;
        }
        printf("native_abi=1 capacity_wakes=%u frames=%u distinct_frames=%u hwdec=videotoolbox nonblack=1 teardown_callbacks=0\n",
               capacity_wakes, frames, distinct_frames);
    }
    return 0;
}
#endif
