// SPDX-License-Identifier: GPL-3.0-or-later
// Diagnostic-only standalone libmpv/OpenGL baseline; not an application frontend.
#include <SDL.h>
#ifdef __APPLE__
#include <OpenGL/OpenGL.h>
#include <OpenGL/gl3.h>
#include <CoreVideo/CoreVideo.h>
#include <IOKit/pwr_mgt/IOPMLib.h>
#endif
#include <mpv/client.h>
#include <mpv/render_gl.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static Uint32 wake_event, checkpoint_event, finish_event;
static atomic_bool event_queued;
static atomic_bool render_pending;
static atomic_uint_fast64_t notifications;
static uint64_t draws;
#ifdef __APPLE__
static atomic_bool clock_ready;
static atomic_bool continuous_clock, clock_idle;
static atomic_bool phase_clock, clock_running;
static atomic_uint empty_ticks;
static atomic_uint_fast64_t clock_ticks;
static uint64_t clock_starts, clock_stops;
static IOPMAssertionID power_assertion = kIOPMNullAssertionID;
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
static CVReturn clock_tick(CVDisplayLinkRef link, const CVTimeStamp *now,
                          const CVTimeStamp *output, CVOptionFlags flags,
                          CVOptionFlags *out_flags, void *ctx);
#pragma clang diagnostic pop
#endif
static int64_t dropped, decoder_dropped;
static int64_t video_width, video_height, audio_rate;
static double video_fps;
static bool observed_playing, observed_paused, hidden, resume_on_show;
static char hwdec[80], audio_output[80], codec[80], audio_codec[80];

static void fail(const char *message) { fprintf(stderr, "%s\n", message); exit(1); }
static void check(int result, const char *message) { if (result < 0) fail(message); }
static void post(Uint32 type) {
    SDL_Event event = {.type = type};
    if (SDL_PushEvent(&event) < 0) abort();
}
static void wake(void *unused) {
    (void)unused;
    if (!atomic_exchange(&event_queued, true)) post(wake_event);
}
#ifdef __APPLE__
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
static CVReturn clock_tick(CVDisplayLinkRef link, const CVTimeStamp *now,
                          const CVTimeStamp *output, CVOptionFlags flags,
                          CVOptionFlags *out_flags, void *ctx) {
    (void)link; (void)now; (void)output; (void)flags; (void)out_flags; (void)ctx;
    atomic_fetch_add(&clock_ticks, 1);
    if (atomic_load(&phase_clock)) {
        if (atomic_load(&render_pending) && !atomic_exchange(&clock_ready, true)) wake(NULL);
        return kCVReturnSuccess;
    }
    if (atomic_load(&continuous_clock) && !atomic_load(&render_pending)) {
        if (atomic_fetch_add(&empty_ticks, 1) >= 1 && !atomic_exchange(&clock_idle, true))
            wake(NULL);
    } else {
        atomic_store(&empty_ticks, 0);
        if (!atomic_exchange(&clock_ready, true)) wake(NULL);
    }
    return kCVReturnSuccess;
}
#pragma clang diagnostic pop
#endif
static void frame(void *unused) {
    (void)unused;
    atomic_fetch_add(&notifications, 1);
    atomic_store(&render_pending, true);
#ifdef __APPLE__
    if (atomic_load(&phase_clock) && atomic_load(&clock_running)) return;
#endif
    wake(NULL);
}
static Uint32 timer(Uint32 interval, void *data) {
    (void)interval;
    post((Uint32)(uintptr_t)data);
    return 0;
}
static void *resolve(void *unused, const char *name) {
    (void)unused;
    return SDL_GL_GetProcAddress(name);
}
static void snapshot(Uint32 elapsed) {
    printf("elapsed_ms=%u notifications=%llu draws=%llu dropped=%lld decoder_dropped=%lld hwdec=%s ao=%s codec=%s audio_codec=%s audio_rate=%lld video=%lldx%lld fps=%.3f playing=%d paused=%d hidden=%d\n",
           elapsed, (unsigned long long)atomic_load(&notifications),
           (unsigned long long)draws, (long long)dropped,
           (long long)decoder_dropped, hwdec, audio_output, codec, audio_codec,
           (long long)audio_rate, (long long)video_width, (long long)video_height,
           video_fps, observed_playing, observed_paused, hidden);
#ifdef __APPLE__
    printf("clock starts=%llu stops=%llu ticks=%llu active=%d prevents_display_sleep=%d\n",
        (unsigned long long)clock_starts, (unsigned long long)clock_stops,
        (unsigned long long)atomic_load(&clock_ticks), atomic_load(&clock_running),
        power_assertion != kIOPMNullAssertionID);
#endif
    fflush(stdout);
}
static void drain(mpv_handle *mpv) {
    for (;;) {
        mpv_event *event = mpv_wait_event(mpv, 0);
        if (event->event_id == MPV_EVENT_NONE) break;
        if (event->event_id == MPV_EVENT_START_FILE || event->event_id == MPV_EVENT_SEEK)
            observed_playing = false;
        if (event->event_id == MPV_EVENT_FILE_LOADED || event->event_id == MPV_EVENT_PLAYBACK_RESTART)
            observed_playing = true;
        if (event->event_id == MPV_EVENT_END_FILE) {
            observed_playing = false;
            mpv_event_end_file *end = event->data;
            printf("end_file reason=%d error=%d\n", end->reason, end->error);
        }
        if (event->event_id != MPV_EVENT_PROPERTY_CHANGE || !event->data) continue;
        mpv_event_property *property = event->data;
        if (!property->data) continue;
        if (event->reply_userdata == 1 && property->format == MPV_FORMAT_INT64)
            dropped = *(int64_t *)property->data;
        if (event->reply_userdata == 2 && property->format == MPV_FORMAT_INT64)
            decoder_dropped = *(int64_t *)property->data;
        if (event->reply_userdata == 6 && property->format == MPV_FORMAT_FLAG)
            observed_paused = *(int *)property->data != 0;
        if ((event->reply_userdata == 7 || event->reply_userdata == 8) &&
            property->format == MPV_FORMAT_FLAG && *(int *)property->data)
            observed_playing = false;
        if (event->reply_userdata == 10 && property->format == MPV_FORMAT_INT64)
            audio_rate = *(int64_t *)property->data;
        if (event->reply_userdata == 11 && property->format == MPV_FORMAT_INT64)
            video_width = *(int64_t *)property->data;
        if (event->reply_userdata == 12 && property->format == MPV_FORMAT_INT64)
            video_height = *(int64_t *)property->data;
        if (event->reply_userdata == 13 && property->format == MPV_FORMAT_DOUBLE)
            video_fps = *(double *)property->data;
        char *dest = event->reply_userdata == 3 ? hwdec :
            event->reply_userdata == 4 ? audio_output : event->reply_userdata == 5 ? codec :
            event->reply_userdata == 9 ? audio_codec : NULL;
        if (dest && property->format == MPV_FORMAT_STRING)
            snprintf(dest, 80, "%s", *(char **)property->data);
    }
}
int main(int argc, char **argv) {
    if ((argc < 5 || argc > 9 || argc == 8) || argv[1][0] != '/')
        fail("usage: media-baseline /absolute/local/clip seconds block_for_target(0|1) timing_offset_seconds [advanced_control(0|1)] [swap_mode(0=SDL,1=native,2=unblocked,3=before-render,4=demand-clock,5=bounded-clock,6=playing-phase)] [width height]");
    int seconds = atoi(argv[2]), block = atoi(argv[3]);
    if (seconds < 5 || seconds > 600 || (block != 0 && block != 1)) fail("invalid diagnostic arguments");
    SDL_SetHint(SDL_HINT_NO_SIGNAL_HANDLERS, "1");
    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_TIMER)) fail(SDL_GetError());
    wake_event = SDL_RegisterEvents(3);
    if (wake_event == (Uint32)-1) fail("SDL event registration failed");
    checkpoint_event = wake_event + 1; finish_event = wake_event + 2;
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_MAJOR_VERSION, 4);
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_MINOR_VERSION, 1);
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_PROFILE_MASK, SDL_GL_CONTEXT_PROFILE_CORE);
    SDL_GL_SetAttribute(SDL_GL_DOUBLEBUFFER, 1);
    int window_width = argc == 9 ? atoi(argv[7]) : 928;
    int window_height = argc == 9 ? atoi(argv[8]) : 280;
    if (window_width < 1 || window_width > 4096 || window_height < 1 || window_height > 4096)
        fail("invalid window dimensions");
    SDL_Window *window = SDL_CreateWindow("Serein media baseline (diagnostic)",
        SDL_WINDOWPOS_CENTERED, SDL_WINDOWPOS_CENTERED, window_width, window_height,
        SDL_WINDOW_OPENGL | SDL_WINDOW_SHOWN | SDL_WINDOW_ALLOW_HIGHDPI);
    if (!window) fail(SDL_GetError());
    SDL_GLContext gl = SDL_GL_CreateContext(window);
    if (!gl) fail(SDL_GetError());
    int swap_mode = argc >= 7 ? atoi(argv[6]) : 0;
    if (swap_mode < 0 || swap_mode > 6) fail("invalid swap mode");
    printf("swap_mode=%d\n", swap_mode);
    printf("swap_interval_set=%d\n", SDL_GL_SetSwapInterval(swap_mode == 0 || swap_mode == 3 ? 1 : 0));
#ifdef __APPLE__
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    GLint interval = swap_mode == 1 ? 1 : 0;
    CGLError swap_result = CGLSetParameter(CGLGetCurrentContext(), kCGLCPSwapInterval, &interval);
    CGLGetParameter(CGLGetCurrentContext(), kCGLCPSwapInterval, &interval);
    printf("native_swap_interval=%d set_result=%d\n", interval, swap_result);
    CVDisplayLinkRef demand_clock = NULL;
    if (swap_mode == 4 || swap_mode == 5 || swap_mode == 6) {
        atomic_store(&continuous_clock, swap_mode == 5);
        atomic_store(&phase_clock, swap_mode == 6);
        if (CVDisplayLinkCreateWithActiveCGDisplays(&demand_clock) != kCVReturnSuccess ||
            CVDisplayLinkSetOutputCallback(demand_clock, clock_tick, NULL) != kCVReturnSuccess ||
            CVDisplayLinkSetCurrentCGDisplayFromOpenGLContext(demand_clock, CGLGetCurrentContext(),
                CGLGetPixelFormat(CGLGetCurrentContext())) != kCVReturnSuccess)
            fail("demand clock creation failed");
    }
#pragma clang diagnostic pop
#endif
    typedef const unsigned char *(*GetGlString)(unsigned int);
    GetGlString get_gl_string = (GetGlString)SDL_GL_GetProcAddress("glGetString");
    if (!get_gl_string) fail("glGetString unavailable");
    printf("GL vendor=%s renderer=%s version=%s\n", get_gl_string(0x1F00),
           get_gl_string(0x1F01), get_gl_string(0x1F02));
    int drawable_width, drawable_height;
    SDL_GL_GetDrawableSize(window, &drawable_width, &drawable_height);
    printf("drawable=%dx%d\n", drawable_width, drawable_height);
    SDL_DisplayMode mode = {0};
    SDL_GetCurrentDisplayMode(SDL_GetWindowDisplayIndex(window), &mode);
    printf("display=%dx%d@%dHz block=%d timing_offset=%s SDL=%d.%d.%d\n",
           mode.w, mode.h, mode.refresh_rate, block, argv[4],
           SDL_MAJOR_VERSION, SDL_MINOR_VERSION, SDL_PATCHLEVEL);
    mpv_handle *mpv = mpv_create();
    if (!mpv) fail("mpv_create failed");
    const char *options[][2] = {
        {"config", "no"}, {"vo", "libmpv"}, {"hwdec", "auto-safe"},
        {"ytdl", "no"}, {"load-scripts", "no"}, {"terminal", "no"},
        {"load-stats-overlay", "no"}, {"load-console", "no"}, {"load-select", "no"},
        {"load-positioning", "no"}, {"load-commands", "no"}, {"load-context-menu", "no"},
        {"load-auto-profiles", "no"}, {"osc", "no"}, {"osd-level", "0"},
        {"sub-auto", "no"}, {"audio-file-auto", "no"}, {"access-references", "no"},
        {"video-sync", "audio"}, {"video-timing-offset", argv[4]}, {"idle", "yes"},
        {"keep-open", "yes"}, {"input-default-bindings", "no"}, {"input-vo-keyboard", "no"},
        {"demuxer-max-bytes", "32MiB"}, {"demuxer-max-back-bytes", "8MiB"},
    };
    for (size_t i = 0; i < sizeof options / sizeof options[0]; ++i)
        check(mpv_set_option_string(mpv, options[i][0], options[i][1]), options[i][0]);
    check(mpv_initialize(mpv), "mpv_initialize failed");
    check(mpv_observe_property(mpv, 1, "frame-drop-count", MPV_FORMAT_INT64), "observe failed");
    check(mpv_observe_property(mpv, 2, "decoder-frame-drop-count", MPV_FORMAT_INT64), "observe failed");
    check(mpv_observe_property(mpv, 3, "hwdec-current", MPV_FORMAT_STRING), "observe failed");
    check(mpv_observe_property(mpv, 4, "current-ao", MPV_FORMAT_STRING), "observe failed");
    check(mpv_observe_property(mpv, 5, "video-codec", MPV_FORMAT_STRING), "observe failed");
    check(mpv_observe_property(mpv, 6, "pause", MPV_FORMAT_FLAG), "observe failed");
    check(mpv_observe_property(mpv, 7, "paused-for-cache", MPV_FORMAT_FLAG), "observe failed");
    check(mpv_observe_property(mpv, 8, "eof-reached", MPV_FORMAT_FLAG), "observe failed");
    check(mpv_observe_property(mpv, 9, "audio-codec-name", MPV_FORMAT_STRING), "observe failed");
    check(mpv_observe_property(mpv, 10, "audio-params/samplerate", MPV_FORMAT_INT64), "observe failed");
    check(mpv_observe_property(mpv, 11, "width", MPV_FORMAT_INT64), "observe failed");
    check(mpv_observe_property(mpv, 12, "height", MPV_FORMAT_INT64), "observe failed");
    check(mpv_observe_property(mpv, 13, "container-fps", MPV_FORMAT_DOUBLE), "observe failed");
    mpv_opengl_init_params init = {.get_proc_address = resolve};
    int advanced = argc >= 6 ? atoi(argv[5]) : 1;
    if (advanced != 0 && advanced != 1) fail("invalid advanced control");
    printf("advanced_control=%d\n", advanced);
    mpv_render_param creation[] = {
        {MPV_RENDER_PARAM_API_TYPE, MPV_RENDER_API_TYPE_OPENGL},
        {MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, &init},
        {MPV_RENDER_PARAM_ADVANCED_CONTROL, &advanced}, {0}
    };
    mpv_render_context *render;
    check(mpv_render_context_create(&render, mpv, creation), "mpv renderer failed");
    mpv_set_wakeup_callback(mpv, wake, NULL);
    mpv_render_context_set_update_callback(render, frame, NULL);
    const char *load[] = {"loadfile", argv[1], "replace", NULL};
    check(mpv_command_async(mpv, 0, load), "load failed");
    Uint32 started = SDL_GetTicks();
    SDL_TimerID t1 = SDL_AddTimer(10000, timer, (void *)(uintptr_t)checkpoint_event);
    SDL_TimerID t2 = SDL_AddTimer(25000, timer, (void *)(uintptr_t)checkpoint_event);
    SDL_TimerID t3 = SDL_AddTimer((Uint32)seconds * 1000, timer, (void *)(uintptr_t)finish_event);
    SDL_TimerID t4 = SDL_AddTimer(70000, timer, (void *)(uintptr_t)checkpoint_event);
    bool running = true;
    while (running) {
        SDL_Event event;
        if (!SDL_WaitEvent(&event)) fail(SDL_GetError());
        if (event.type == finish_event || event.type == SDL_QUIT) running = false;
        if (event.type == checkpoint_event || !running) snapshot(SDL_GetTicks() - started);
        if (event.type == wake_event) {
            atomic_store(&event_queued, false);
            drain(mpv);
        }
        if (swap_mode == 6 && event.type == SDL_WINDOWEVENT) {
            bool was_hidden = hidden;
            if (event.window.event == SDL_WINDOWEVENT_HIDDEN || event.window.event == SDL_WINDOWEVENT_MINIMIZED)
                hidden = true;
            if (event.window.event == SDL_WINDOWEVENT_SHOWN || event.window.event == SDL_WINDOWEVENT_RESTORED)
                hidden = false;
            if (hidden && !was_hidden) {
                resume_on_show = !observed_paused;
                int paused = 1;
                check(mpv_set_property_async(mpv, 0, "pause", MPV_FORMAT_FLAG, &paused), "hidden pause failed");
            } else if (!hidden && was_hidden && resume_on_show) {
                int paused = 0;
                check(mpv_set_property_async(mpv, 0, "pause", MPV_FORMAT_FLAG, &paused), "visible resume failed");
                resume_on_show = false;
            }
        }
        bool redraw = event.type == SDL_WINDOWEVENT && event.window.event == SDL_WINDOWEVENT_EXPOSED;
        bool may_render = true;
#ifdef __APPLE__
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        if (demand_clock && swap_mode == 6) {
            bool playing = running && observed_playing && !observed_paused && !hidden;
            if (playing && power_assertion == kIOPMNullAssertionID) {
                if (IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleDisplaySleep,
                    kIOPMAssertionLevelOn, CFSTR("Serein standalone playback diagnostic"),
                    &power_assertion) != kIOReturnSuccess) fail("playback power assertion failed");
            } else if (!playing && power_assertion != kIOPMNullAssertionID) {
                if (IOPMAssertionRelease(power_assertion) != kIOReturnSuccess)
                    fail("playback power assertion release failed");
                power_assertion = kIOPMNullAssertionID;
            }
            if (!playing) {
                if (CVDisplayLinkIsRunning(demand_clock)) {
                    if (CVDisplayLinkStop(demand_clock) != kCVReturnSuccess)
                        fail("inactive clock stop failed");
                    ++clock_stops;
                }
                atomic_store(&clock_running, false);
                atomic_store(&clock_ready, atomic_load(&render_pending));
            } else if (atomic_load(&render_pending) && !atomic_load(&clock_ready) &&
                       !CVDisplayLinkIsRunning(demand_clock)) {
                if (CVDisplayLinkStart(demand_clock) != kCVReturnSuccess)
                    fail("playing clock start failed");
                atomic_store(&clock_running, true);
                ++clock_starts;
            }
        }
        if (demand_clock && atomic_exchange(&clock_idle, false) && !atomic_load(&render_pending)) {
            if (CVDisplayLinkIsRunning(demand_clock)) {
                if (CVDisplayLinkStop(demand_clock) != kCVReturnSuccess)
                    fail("idle clock stop failed");
                atomic_store(&clock_ready, false);
                ++clock_stops;
            }
        }
        if (demand_clock && atomic_load(&render_pending)) {
            if (atomic_load(&clock_ready)) {
                if (swap_mode == 4) {
                    if (CVDisplayLinkStop(demand_clock) != kCVReturnSuccess)
                        fail("demand clock stop failed");
                    ++clock_stops;
                }
                atomic_store(&clock_ready, false);
            } else {
                if (swap_mode != 6 && !CVDisplayLinkIsRunning(demand_clock)) {
                    atomic_store(&empty_ticks, 0);
                    if (CVDisplayLinkStart(demand_clock) != kCVReturnSuccess)
                        fail("demand clock start failed");
                    ++clock_starts;
                }
                may_render = false;
            }
        }
#pragma clang diagnostic pop
#endif
        if (swap_mode == 6 && hidden) may_render = false;
        if (may_render && atomic_exchange(&render_pending, false))
            redraw |= (mpv_render_context_update(render) & MPV_RENDER_UPDATE_FRAME) != 0;
        if (!may_render) redraw = false;
        if (redraw && running) {
            int width, height;
            SDL_GL_GetDrawableSize(window, &width, &height);
            mpv_opengl_fbo fbo = {.fbo = 0, .w = width, .h = height};
            int flip = 1;
            mpv_render_param parameters[] = {
                {MPV_RENDER_PARAM_OPENGL_FBO, &fbo}, {MPV_RENDER_PARAM_FLIP_Y, &flip},
                {MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME, &block}, {0}
            };
            // Mode3 tests vblank-gated render delivery without delaying the
            // completed frame on the render thread. The first swap waits for
            // SDL's CVDisplayLink; the second publishes the newly drawn frame.
            if (swap_mode == 3) SDL_GL_SwapWindow(window);
            check(mpv_render_context_render(render, parameters), "render failed");
            if (swap_mode == 3) SDL_GL_SetSwapInterval(0);
            SDL_GL_SwapWindow(window);
            if (swap_mode == 3) SDL_GL_SetSwapInterval(1);
            ++draws;
        }
    }
    SDL_RemoveTimer(t1); SDL_RemoveTimer(t2); SDL_RemoveTimer(t3); SDL_RemoveTimer(t4);
#ifdef __APPLE__
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    if (demand_clock) {
        if (CVDisplayLinkIsRunning(demand_clock)) CVDisplayLinkStop(demand_clock);
        atomic_store(&clock_running, false);
        CVDisplayLinkRelease(demand_clock);
    }
    if (power_assertion != kIOPMNullAssertionID) IOPMAssertionRelease(power_assertion);
#pragma clang diagnostic pop
#endif
    printf("clock starts=%llu stops=%llu ticks=%llu\n",
        (unsigned long long)clock_starts, (unsigned long long)clock_stops,
        (unsigned long long)atomic_load(&clock_ticks));
    mpv_set_wakeup_callback(mpv, NULL, NULL);
    mpv_render_context_set_update_callback(render, NULL, NULL);
    mpv_render_context_free(render);
    mpv_terminate_destroy(mpv);
    SDL_GL_DeleteContext(gl); SDL_DestroyWindow(window); SDL_Quit();
    return 0;
}
