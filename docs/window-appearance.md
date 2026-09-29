# Window chrome and appearance

The macOS application places its shared 48px Slint header inside the native
window's titlebar area. The title is hidden, the titlebar is transparent, and
`FullSizeContentView` lets search and application controls use the top of the
window without a separate default title row. The leading 88px is reserved for
the real AppKit traffic lights. Their centers are aligned to the shared header on the main thread. AppKit retains
their native horizontal spacing, hover, accessibility and close/minimize/fullscreen
behavior, as well as the rounded system frame. The application does not draw replacement traffic-light glyphs
or reparent AppKit's titlebar subviews. The adapter borrows the three standard
buttons and their existing ancestors for a synchronous call. It extends the two
native titlebar view frames to the 48px header and converts the content-space
center into button-parent coordinates with public NSView APIs. Native hierarchy
shape is still an implementation assumption: unexpected/missing ancestors cause
alignment to be skipped. No native view pointer survives the call, and relevant
resize/mode observations reapply alignment without polling.

Windows and Linux currently retain their native desktop decoration policy;
integrating the shared header with those native system buttons requires a
separately validated adapter. Rounded corners are not promised on desktops that
do not provide them. The browsing UI, search, sidebar and player remain one
compiled Slint component library. Native decorations do not create another
player, window or presenter.

Slint's `Window.no-frame` remains the source of truth for decorations. It is false
at normal startup. Picture-in-picture snapshots the existing native frame,
sets `no-frame` true, then restores the saved decoration state and geometry on
exit. Winit 0.30.13 drops `FullSizeContentView` when it restores decorations, so
the macOS adapter restores that single style-mask bit after the normal titled
frame returns. Fullscreen follows the native window manager. The shared empty
header region starts a native window drag, while Slint controls and traffic
lights retain their input. Native titlebar double-click, edge resize, minimize,
zoom/maximize, close and the macOS green-button fullscreen behavior remain owned
by AppKit. The existing Slint close-request handler still handles shutdown and
PiP restoration. The native-child media diagnostic retains its ordinary frame.

At the user's explicit request, translucency and blur are requested by default
for this feature batch, overriding SPEC's original opaque/no-glass design
direction. Both remain session-local settings with an immediate opaque opt-out.
Actual alpha styling is gated by the backend's translucency capability. The
whole canvas and shared surface tokens use restrained neutral alpha; text and
accents stay opaque. The header is 48 logical pixels high with a 38px rounded
search field. Decoded video retains its opaque black-backed rectangle; native
blur is not simulated by image textures or CPU video copies.

Blur is a separate native request, applied only where exposed and only with
translucency enabled. It is suspended in fullscreen/global PiP and restored on
leaving those modes. The native-child diagnostic disables both appearance
effects. Changes preserve the native window, media player, graphics context,
and borrowed-texture ownership.

| Backend | Translucency request | Native blur |
| --- | --- | --- |
| macOS | Alpha-capable FemtoVG configuration and native transparent window; requested by default | Experimental Winit request, requested by default when available |
| Windows | Requested; graphics/compositor behavior unqualified | Unavailable through selected Winit API |
| Linux/X11 | Alpha visual requested at window creation; requires compositor | Unavailable through selected Winit API |
| Native Wayland | Requested; compositor behavior unqualified | Disabled: selected API cannot confirm KWin blur-protocol availability |

These describe code/API capabilities, not runtime support claims. Winit exposes
no result or active-state getter for blur. The UI therefore does not report
successful blur activation. The macOS implementation in Winit 0.30.13 uses the
private `CGSSetWindowBackgroundBlurRadius` function, rather than an AppKit public
visual-effect material. This is an experimental compatibility limitation and
must be reviewed before distribution/OS-support claims. System accessibility
and compositor settings can affect the appearance; an opaque option and an
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
  platform restrictions, borderless resizable style, decoration-mask restoration,
  and native blur internals; `src/platform/macos.rs` provides the creation-time
  transparent/hidden-title/full-size-content attributes.
- Already-locked objc2 0.6.4 and objc2-app-kit 0.3.2, now direct macOS-only
  dependencies with limited AppKit features: `MainThreadMarker`, `NSView.window`,
  and public `NSWindow` style/title/standard-button methods; `NSButton` and
  `NSControl` features permit accessing the real controls. The adapter borrows Winit's live view
  synchronously on the main thread and retains no raw handle across events.

`window_chrome.rs` owns the native adapter. `BackendSelector` installs its window
attributes hook before component creation, because X11 cannot add a transparent
visual afterward. Shared Slint bindings own alpha and decoration properties. On
macOS the pinned backend derives native transparency from `Window.background` and `no-frame`,
overriding the creation hook. With translucency enabled the Window background
is transparent and the shared canvas provides one neutral tint, while the
native frame/rounding remains owned by AppKit. Turning translucency off restores
opaque shared tokens and the opaque Window background; this is not a promise
of an AppKit vibrancy material.
Native blur writes are cached and updated only by user actions or observed
window/mode changes. No appearance polling loop is introduced.

The previous 56px integrated-header checkpoint is source
`c75af56c87145c37769c46e9266f3cf6257029d8`; its formatting, strict Clippy and
locked workspace build passed. The current native traffic-light centering, 48px header and whole-window alpha
changes have not been visually qualified on a native display. A fresh isolated native shell attempt exited 0 and produced a
[shared-header snapshot](evidence/2026-09-29-header-shell.png), while presenter
setup still reported native display-clock error `-6661`. The inspected image
shows shared controls and omits native decorations, so cannot verify traffic
lights or compositor appearance. No resource measurement is claimed.

Source review alone does not qualify native appearance, accessibility, compositor,
energy, or performance behavior. Remaining qualification includes resize/drag,
maximize/minimize, close/PiP restoration, multi-monitor scale changes, fullscreen,
screen-reader names/focus, opaque video composition, and system reduced-
transparency behavior on each target.

Shared menu/account controls are 40px high, the badge is 26px and search is 38px;
each is vertically centered in the same 48px header. Headless shared-UI geometry
can verify these bindings, but does not verify AppKit layout or mouse routing.
The native adapter uses [standard window buttons](https://developer.apple.com/documentation/appkit/nswindow)
and [NSView coordinate conversion](https://developer.apple.com/documentation/appkit/nsview/convert%28_%3Afrom%3A%29-1dq9l?language=objc).
