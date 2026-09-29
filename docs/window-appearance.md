# Window chrome and appearance

The normal application uses a shared Slint titlebar with native drag, minimize,
maximize/restore, and close actions. The titlebar is part of the same compiled
component library on each platform. It does not create another application
window or video presenter. The native-child media diagnostic retains native
decorations. Fullscreen and picture-in-picture hide the normal titlebar.

The titlebar's close action dispatches Slint's `CloseRequested` event, preserving
the existing host close handler and PiP restoration behavior. Maximize and
minimize update Slint's window state, rather than writing a competing native
property. The shared titlebar uses Slint's built-in `WindowMoveArea` for native
drag, without a drag timer or global cursor model. Pinned Slint implements
frameless edge resize hit testing on other desktop backends. macOS uses its
native outer resize border: Winit retains `NSWindowStyleMask::Resizable` for a
borderless window. Its `drag_resize_window` method is unsupported on macOS, so
the Slint inner resize strip is disabled there. Native window-manager behavior
still needs platform qualification.

Translucency is an explicit session-local option, initially off. It applies to
the shell chrome; the main browsing surface and video remain opaque. Blur is a
separate opt-in native effect, enabled only with translucency. It is suspended
in fullscreen/PiP and restored on leaving that mode. Appearance changes preserve
the native window, media player, graphics context, and borrowed-texture ownership.

| Backend | Translucency request | Native blur |
| --- | --- | --- |
| macOS | Alpha-capable FemtoVG configuration and native transparent window | Experimental Winit request, off initially |
| Windows | Requested; graphics/compositor behavior unqualified | Unavailable through selected Winit API |
| Linux/X11 | Alpha visual requested at window creation; requires compositor | Unavailable through selected Winit API |
| Native Wayland | Requested; compositor behavior unqualified | Disabled: selected API cannot confirm KWin blur-protocol availability |

These describe code/API capabilities, not runtime support claims. Winit exposes
no result or active-state getter for blur. The UI therefore does not report
successful blur activation. The macOS implementation in Winit 0.30.13 uses the
private `CGSSetWindowBackgroundBlurRadius` function, rather than an AppKit public
visual-effect material. This is an experimental compatibility limitation and
must be reviewed before distribution/OS-support claims. System accessibility
and compositor settings can affect the appearance; an opaque default and an
immediate opt-out remain available.

Source reviewed for this implementation:

- Slint revision `cf3b07d4917e6759a63b0c03913a2594ec653414`,
  `api/rs/slint/lib.rs` and `internal/core/api.rs`: exported Winit accessor,
  window state operations, and close-event dispatch.
- The same Slint revision, `internal/backends/winit/winitwindowadapter.rs` and
  `drag_resize_window.rs`: `no-frame` owns decorations, macOS transparency
  follows window background/no-frame, and edge resize is handled by the backend.
- The same revision, `internal/backends/winit/renderer/femtovg/glcontext.rs`:
  transparency-capable OpenGL configuration is requested on macOS and preferred
  when choosing other platform configurations; opaque fallback can occur.
- Locked Winit 0.30.13, `src/window.rs` and
  `src/platform_impl/macos/window_delegate.rs`: native operation contracts,
  platform restrictions, borderless resizable style, and native blur internals.

`window_chrome.rs` owns the native adapter. `BackendSelector` installs its window
attributes hook before component creation, because X11 cannot add a transparent
visual afterward. Shared Slint bindings own alpha and decoration properties.
Native blur writes are cached and updated only by user actions or observed
window/mode changes. No appearance polling loop is introduced.

This feature pass does not run native, screenshot, accessibility, compositor,
energy, or performance checks. Remaining qualification includes resize/drag,
maximize/minimize, close/PiP restoration, multi-monitor scale changes, fullscreen,
screen-reader names/focus, opaque video composition, and system reduced-
transparency behavior on each target.
