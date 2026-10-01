// SPDX-License-Identifier: GPL-3.0-or-later
// Headless paused-load/VO lifetime qualification, using the installed backend.
#define OXPLAY_METAL_SMOKE_HELPERS_ONLY
#include "macos_smoke.m"

static bool advanced_mode;

struct harness {
    mpv_handle *mpv;
    mpv_render_context *context;
    id<MTLDevice> device;
    id<MTLCommandQueue> queue;
    id<MTLTexture> texture;
    mpv_render_param *render;
    int *busy;
};

static bool stage(struct harness *h, const char *path, bool video, bool paused,
                  unsigned index)
{
    const char *load[] = {"loadfile", path, "replace", "-1",
                         video ? "start=1" : "start=0", NULL};
    int pause = paused;
    if (!check(mpv_command_async(h->mpv, index, load), "load") ||
        !check(mpv_set_property_async(h->mpv, index + 100, "pause", MPV_FORMAT_FLAG, &pause), "pause"))
        return false;
    bool loaded = false, restarted = false;
    unsigned frame_updates = 0, nonblack = 0, render_wakes = atomic_load(&wakes);
    double deadline = seconds() + 5;
    while (seconds() < deadline) {
        mpv_event *event;
        while ((event = mpv_wait_event(h->mpv, 0))->event_id != MPV_EVENT_NONE) {
            if ((event->event_id == MPV_EVENT_COMMAND_REPLY ||
                 event->event_id == MPV_EVENT_SET_PROPERTY_REPLY) && event->error < 0)
                return check(event->error, "async command reply");
            if (event->event_id == MPV_EVENT_START_FILE) {
                loaded = restarted = false;
                frame_updates = nonblack = 0;
            }
            loaded |= event->event_id == MPV_EVENT_FILE_LOADED;
            restarted |= loaded && event->event_id == MPV_EVENT_PLAYBACK_RESTART;
            if (event->event_id == MPV_EVENT_END_FILE &&
                ((mpv_event_end_file *)event->data)->error < 0)
                return false;
        }
        // Always serve the render dispatch, including while paused/audio-only.
        // No display link or UI admission gate is present in this isolation.
        if (mpv_render_context_update(h->context) & MPV_RENDER_UPDATE_FRAME) {
            frame_updates++;
            if (!check(mpv_render_context_render(h->context, h->render), "render"))
                return false;
            if (!*h->busy) {
                nonblack += pixels_hash(h->device, h->queue, h->texture) != 0;
                if (advanced_mode)
                    mpv_render_context_report_swap(h->context);
            }
        }
        if (loaded && restarted && (!video || nonblack)) {
            int actual_pause = 0;
            int64_t decoder_width = 0;
            int width_result = mpv_get_property(h->mpv, "video-dec-params/w", MPV_FORMAT_INT64, &decoder_width);
            check(mpv_get_property(h->mpv, "pause", MPV_FORMAT_FLAG, &actual_pause), "pause evidence");
            char *hardware = mpv_get_property_string(h->mpv, "hwdec-current");
            bool vt = hardware && !strcmp(hardware, "videotoolbox");
            mpv_free(hardware);
            bool valid = actual_pause == paused &&
                (video ? width_result >= 0 && decoder_width > 0 && vt : width_result == MPV_ERROR_PROPERTY_UNAVAILABLE);
            printf("handoff_stage=%u video=%d paused=%d loaded=%d restarted=%d frame_updates=%u nonblack=%u decoder_width=%lld videotoolbox=%d render_wakes=%u valid=%d\n",
                   index, video, actual_pause, loaded, restarted, frame_updates, nonblack,
                   (long long)decoder_width, vt, atomic_load(&wakes) - render_wakes, valid);
            return valid;
        }
        tick();
    }
    fprintf(stderr, "handoff_timeout stage=%u loaded=%d restarted=%d frame_updates=%u nonblack=%u\n",
            index, loaded, restarted, frame_updates, nonblack);
    return false;
}

int main(int argc, const char *argv[])
{
    if ((argc != 3 && argc != 4) || argv[1][0] != '/' || argv[2][0] != '/' ||
        (argc == 4 && strcmp(argv[3], "--advanced")))
        return 2;
    advanced_mode = argc == 4;
    @autoreleasepool {
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        id<MTLCommandQueue> queue = [device newCommandQueue];
        MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:
            MTLPixelFormatRGBA8Unorm width:320 height:180 mipmapped:NO];
        desc.usage = MTLTextureUsageShaderRead | MTLTextureUsageRenderTarget;
        desc.storageMode = MTLStorageModePrivate;
        id<MTLTexture> texture = [device newTextureWithDescriptor:desc];
        mpv_handle *mpv = mpv_create();
        if (!device || !queue || !texture || !mpv)
            return 3;
        for (const char **p = (const char *[]) {"config", "no", "terminal", "no", "msg-level", "all=no",
             "vo", "libmpv", "hwdec", "videotoolbox", "ao", "null", "keep-open", "yes", "idle", "yes", NULL}; *p; p += 2)
            if (!check(mpv_set_option_string(mpv, p[0], p[1]), "option"))
                return 4;
        if (!check(mpv_initialize(mpv), "initialize"))
            return 4;
        mpv_metal_init_params init = {.metal_device = (__bridge void *)device,
            .command_queue = (__bridge void *)queue, .wakeup = wake};
        int advanced = advanced_mode, block = 0, busy = 0;
        mpv_render_param create[] = {{MPV_RENDER_PARAM_API_TYPE, "metal"},
            {MPV_RENDER_PARAM_METAL_INIT_PARAMS, &init}, {MPV_RENDER_PARAM_ADVANCED_CONTROL, &advanced}, {0}};
        if (!advanced_mode)
            create[2] = (mpv_render_param){0};
        mpv_render_context *context = NULL;
        if (!check(mpv_render_context_create(&context, mpv, create), "create"))
            return 5;
        mpv_render_context_set_update_callback(context, wake, NULL);
        mpv_metal_texture target = {.texture = (__bridge void *)texture, .width = 320, .height = 180};
        mpv_render_param render[] = {{MPV_RENDER_PARAM_METAL_TEXTURE, &target},
            {MPV_RENDER_PARAM_METAL_BUSY, &busy}, {MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME, &block}, {0}};
        struct harness h = {mpv, context, device, queue, texture, render, &busy};
        printf("native_handoff_advanced_control=%d report_swap=%d\n", advanced_mode, advanced_mode);
        bool valid = stage(&h, argv[1], true, false, 1) &&
                     stage(&h, argv[2], false, true, 2) &&
                     stage(&h, argv[1], true, true, 3);
        const char *stop[] = {"stop", NULL};
        valid &= check(mpv_command(mpv, stop), "stop");
        double drain_deadline = seconds() + 0.5;
        while (seconds() < drain_deadline) {
            mpv_render_context_update(context);
            while (mpv_wait_event(mpv, 0)->event_id != MPV_EVENT_NONE) {}
            tick();
        }
        valid &= stage(&h, argv[1], true, true, 4);
        mpv_render_context_set_update_callback(context, NULL, NULL);
        mpv_render_context_free(context);
        mpv_terminate_destroy(mpv);
        return valid ? 0 : 10;
    }
}
