// SPDX-License-Identifier: GPL-3.0-or-later
// Experimental video surface only: no application controls, event ownership,
// contentView replacement, or media commands. All entry points are main-thread.
#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <OpenGL/OpenGL.h>
#include <math.h>
#include <stdlib.h>
#include <stdint.h>

@interface OxplayVideoClip : NSView
@end
@implementation OxplayVideoClip
- (BOOL)isFlipped { return YES; }
- (BOOL)acceptsFirstResponder { return NO; }
- (NSView *)hitTest:(NSPoint)point { (void)point; return nil; }
@end

@interface OxplayVideoView : NSView
@end
@implementation OxplayVideoView
- (BOOL)isFlipped { return YES; }
- (BOOL)acceptsFirstResponder { return NO; }
- (NSView *)hitTest:(NSPoint)point { (void)point; return nil; }
// AppKit exposure never calls libmpv or introduces a second render owner.
- (void)drawRect:(NSRect)rect { (void)rect; }
@end

@interface OxplayVideoSurface : NSObject
@property(strong) NSView *parent;
@property(strong) OxplayVideoClip *clip;
@property(strong) OxplayVideoView *video;
@property(strong) NSOpenGLContext *context;
@end
@implementation OxplayVideoSurface
@end

int oxplay_child_is_main(void) { return [NSThread isMainThread] ? 1 : 0; }

void *oxplay_child_create(void *parentPointer) {
    if (![NSThread isMainThread] || !parentPointer) return NULL;
    CGLContextObj previous = CGLGetCurrentContext();
    @autoreleasepool {
      OxplayVideoSurface *surface = nil;
      @try {
        NSView *parent = (__bridge NSView *)parentPointer;
        if (!parent.window) return NULL;
        NSOpenGLPixelFormatAttribute attrs[] = {
            NSOpenGLPFAOpenGLProfile, NSOpenGLProfileVersion4_1Core,
            NSOpenGLPFADoubleBuffer, NSOpenGLPFAAccelerated,
            NSOpenGLPFANoRecovery, NSOpenGLPFAColorSize, 24,
            NSOpenGLPFAAlphaSize, 8, 0
        };
        NSOpenGLPixelFormat *format = [[NSOpenGLPixelFormat alloc] initWithAttributes:attrs];
        if (!format) return NULL;
        surface = [OxplayVideoSurface new];
        surface.parent = parent;
        surface.clip = [[OxplayVideoClip alloc] initWithFrame:NSMakeRect(0, 0, 1, 1)];
        surface.clip.wantsLayer = YES;
        surface.clip.layer.masksToBounds = YES;
        surface.clip.hidden = YES;
        surface.video = [[OxplayVideoView alloc] initWithFrame:NSMakeRect(0, 0, 1, 1)];
        if (!surface.video) return NULL;
        surface.video.wantsLayer = YES;
        surface.video.wantsBestResolutionOpenGLSurface = YES;
        surface.context = [[NSOpenGLContext alloc] initWithFormat:format shareContext:nil];
        if (!surface.context || !surface.context.CGLContextObj) return NULL;
        [surface.clip addSubview:surface.video];
        [parent addSubview:surface.clip];
        // Plain NSView has no implicit OpenGL update/reshape/draw callbacks.
        // Set layer backing before associating the explicit context, as glutin
        // 0.32.3 does for its CGL NSView surface.
        [surface.context setView:surface.video];
        return (__bridge_retained void *)surface;
      } @catch (NSException *exception) {
        (void)exception;
        @try { [surface.context clearDrawable]; [surface.clip removeFromSuperview]; }
        @catch (NSException *cleanupException) { (void)cleanupException; abort(); }
        return NULL;
      } @finally { if (CGLSetCurrentContext(previous) != kCGLNoError) abort(); }
    }
}

void *oxplay_child_context(void *opaque) {
    if (![NSThread isMainThread] || !opaque) return NULL;
    return [(__bridge OxplayVideoSurface *)opaque context].CGLContextObj;
}

int oxplay_child_window_number(void *opaque, int64_t *number) {
    if (![NSThread isMainThread] || !opaque || !number) return -1;
    @try {
        // Read only this retained surface's owning window. Never enumerate
        // other windows, inspect titles, or capture any pixels here.
        NSWindow *window = [(__bridge OxplayVideoSurface *)opaque parent].window;
        if (!window || window.windowNumber <= 0) return -1;
        *number = (int64_t)window.windowNumber;
        return 0;
    } @catch (NSException *exception) { (void)exception; return -1; }
}

int oxplay_child_geometry(void *opaque, double x, double y, double width, double height,
                          double clipX, double clipY, double clipWidth, double clipHeight,
                          int *pixelWidth, int *pixelHeight) {
    if (![NSThread isMainThread] || !opaque) return -1;
    @autoreleasepool { @try {
        OxplayVideoSurface *surface = (__bridge OxplayVideoSurface *)opaque;
        surface.clip.hidden = YES;
        NSRect proposedBacking = [surface.parent convertRectToBacking:NSMakeRect(x, y, width, height)];
        NSRect proposedClip = [surface.parent convertRectToBacking:NSMakeRect(clipX, clipY, clipWidth, clipHeight)];
        // Validate before resizing a layer/drawable, not after a large native
        // allocation has already happened. No silent quality downscaling.
        if (NSWidth(proposedBacking) < 1 || NSHeight(proposedBacking) < 1 ||
            NSWidth(proposedBacking) > 4096 || NSHeight(proposedBacking) > 4096 ||
            NSWidth(proposedClip) < 1 || NSHeight(proposedClip) < 1 ||
            NSWidth(proposedClip) > 4096 || NSHeight(proposedClip) > 4096) return -1;
        // Input coordinates are logical top-left window content coordinates.
        // Winit is flipped today; do not silently depend on that fact here.
        CGFloat parentY = surface.parent.isFlipped ? clipY :
            NSHeight(surface.parent.bounds) - clipY - clipHeight;
        surface.clip.frame = NSMakeRect(clipX, parentY, clipWidth, clipHeight);
        surface.video.frame = NSMakeRect(x - clipX, y - clipY, width, height);
        [surface.context update];
        NSRect backing = [surface.video convertRectToBacking:surface.video.bounds];
        *pixelWidth = (int)llround(NSWidth(backing));
        *pixelHeight = (int)llround(NSHeight(backing));
        return 0;
    } @catch (NSException *exception) { (void)exception; return -1; } }
}

int oxplay_child_hidden(void *opaque, int hidden) {
    if (![NSThread isMainThread] || !opaque) return -1;
    @try { [(__bridge OxplayVideoSurface *)opaque clip].hidden = hidden != 0; return 0; }
    @catch (NSException *exception) { (void)exception; return -1; }
}

int oxplay_child_flush(void *opaque) {
    if (![NSThread isMainThread] || !opaque) return -1;
    @try { [[(__bridge OxplayVideoSurface *)opaque context] flushBuffer]; return 0; }
    @catch (NSException *exception) { (void)exception; return -1; }
}

void oxplay_child_destroy(void *opaque) {
    if (!opaque) return;
    // Rust !Send ownership guarantees this; do not dispatch synchronously from
    // another thread and create a shutdown dependency cycle.
    if (![NSThread isMainThread]) abort();
    CGLContextObj previous = CGLGetCurrentContext();
    @autoreleasepool {
        OxplayVideoSurface *surface = (__bridge_transfer OxplayVideoSurface *)opaque;
        // No Objective-C exception may cross the Rust FFI boundary.
        @try {
            surface.clip.hidden = YES;
            [surface.context clearDrawable];
            [surface.clip removeFromSuperview];
        } @catch (NSException *exception) { (void)exception; abort(); }
        @finally { if (CGLSetCurrentContext(previous) != kCGLNoError) abort(); }
    }
}
