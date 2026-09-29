# Window chrome and appearance

The normal application uses the operating system's native window frame and
controls. On macOS this means AppKit traffic lights and system-rounded frame
corners, rather than shared minimize/maximize/close glyphs inside a square
borderless window. Windows and Linux use their native desktop decoration policy;
rounded corners are not promised on desktops that do not provide them. The
browsing UI, search, sidebar and player remain one compiled Slint component
library. Native decorations do not create another player, window or presenter.

Slint's `Window.no-frame` remains the source of truth for decorations. It is false
at normal startup. Picture-in-picture snapshots the existing native frame,
sets `no-frame` true, then restores the saved decoration state and geometry on
exit. Fullscreen follows the native window manager; Winit restores the normal
frame when fullscreen ends. The main app no longer reserves a shared custom
titlebar row or draws substitute system buttons. Drag, edge resize, minimize,
zoom/maximize, close and the macOS green-button fullscreen behavior belong to
the native frame. The existing Slint close-request handler still handles
shutdown and PiP restoration.

Translucency is an explicit session-local option, initially off. It applies to
the shell background and sidebar; the main browsing surface and video remain
opaque. Blur is a separate opt-in native effect, enabled only with translucency. It is suspended
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
visual afterward. Shared Slint bindings own alpha and decoration properties. On
macOS the pinned backend derives native transparency from `Window.background` and `no-frame`,
overriding the creation hook. Keeping the normal background opaque therefore
preserves the native titlebar paint and rounded frame. Opting into translucency
can also make the native titlebar transparent; this is the selected upstream
behavior, not a promise of an AppKit vibrancy material.
Native blur writes are cached and updated only by user actions or observed
window/mode changes. No appearance polling loop is introduced.

Source review alone does not qualify native appearance, accessibility, compositor,
energy, or performance behavior. Remaining qualification includes resize/drag,
maximize/minimize, close/PiP restoration, multi-monitor scale changes, fullscreen,
screen-reader names/focus, opaque video composition, and system reduced-
transparency behavior on each target.
