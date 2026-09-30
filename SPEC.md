# Native YouTube Client — Product and Engineering Specification

**Document version:** 2.0 — Slint edition  
**Updated:** October 1, 2026 (explicit-download amendment)  
**Status:** Implementation contract; no implementation or benchmark results are claimed.  
**Audience:** Maintainers, contributors, and the implementing AI agent.  
**Working title:** Native YouTube Client. Final naming and brand clearance are separate tasks.

## Revision scope

This revision supersedes the GPUI-specific frontend and dependency requirements in version 1.0. The chosen architecture is **one shared, compiled Slint UI + an in-process Rust core + replaceable media/presentation adapters**. It preserves the account, advertisement-suppression, privacy, clean YouTube-inspired design, and resource-budget requirements. It adds explicit rendering/invalidation tests, a low-copy video gate, Slint-specific model/lifetime rules, and a revised licensing decision. No application implementation or measured performance is implied by this document.

The October 1, 2026 amendment records a project-owner decision: explicit, user-initiated downloads of guest-accessible public videos are now in scope under the constraints of Section 11.1. Playback buffering remains transient; everything else in this revision is unchanged.

## 1. Product definition

Build an open-source, privacy-focused desktop YouTube client with a Rust application core, a **single cross-platform Slint frontend built from reviewed upstream source**, native video playback, low idle overhead, and hardware-accelerated decoding. The application should feel immediately familiar to a YouTube user: prominent search, a restrained navigation sidebar, thumbnail grids, channel pages, and a clean watch page.

The product must support optional connection to the user's real YouTube account and suppress supported YouTube-served advertisements. These are actual requirements, not decorative UI elements. A guest-only preview may ship before account support, but it must not be described as the completed product.

### 1.1 Priority order

When requirements conflict, prioritize credential safety and honest behavior first, reliable playback second, resource efficiency third, and visual polish fourth. Slint is the chosen application frontend; an integration difficulty is not permission to switch frameworks silently. Development speed is not the optimization target: prefer measured resource efficiency and maintainable integration over the quickest demonstration. Platform-specific media glue is allowed; separate platform-specific application UIs are not the chosen architecture.

**MUST** identifies a release requirement. **SHOULD** identifies a preferred approach that can change through an architecture decision record (ADR). **MAY** identifies optional scope. Performance numbers are proposed engineering budgets, not statements about existing performance.

### 1.2 Assumptions and platform scope

Target Windows, macOS, and Linux. Validate Linux Wayland and X11 separately. Start with the operating system and GPU actually available to the implementing agent; record that choice. Complete one real playback path before attempting several platform implementations in parallel.

A first stable release may support one validated platform. Other platforms must remain explicitly experimental until their playback, account, security, accessibility, and resource tests pass. A successful cross-compilation is not proof of runtime support. The complete cross-platform product still targets all three desktop OS families; a one-platform release must not be advertised as completing that scope. Mobile, television, and browser builds are outside version 1.

### 1.3 Non-goals for version 1

Do not implement a new video decoder, a general-purpose browser, a cloud backend, an advertising business, uploads, live chat, casting, or a plugin marketplace. Explicit user-initiated downloads are in scope only as constrained by Section 11.1; background, bulk, or account-authorized downloading is not. DRM bypass, access to content without authorization, account challenge bypass, and defeating age or purchase requirements are not product features. Creator-embedded sponsorship skipping is separate from YouTube ad suppression and may be considered later.

## 2. Non-negotiable technology decisions

### 2.1 Application, UI, and runtime

Use Rust for maintained application logic and **Slint for all application screens and player controls**. Keep layouts, visual components, and style tokens in `.slint` files compiled through `slint-build` and included in the Rust application. This build-time integration produces Rust code; a production UI interpreter is not required. [S1]

Do not use Electron, Tauri, Chromium/CEF, an HTML YouTube player, or an embedded website as the frontend or normal playback mechanism. Do not create separate SwiftUI, WinUI, GTK, or other per-OS browsing interfaces. Small native adapters for video surfaces, OS dialogs, credentials, and system integration are permitted behind explicit interfaces.

The Rust core is an **in-process library**, not a separate local HTTP server or a persistent background daemon. Use an existing media engine, with **libmpv as the initial candidate**, and an established extractor, with **yt-dlp as the initial candidate**, behind replaceable interfaces. Native dependencies and short-lived helper processes are allowed; do not rewrite reliable decoders just to obtain an entirely Rust dependency tree.

Do not add a persistent Node.js, Python, browser, or token-provider service by default. Any required helper must be supervised, bounded, documented, and included in measurements. Build-time tooling is distinct from a runtime dependency; do not ship preview tools or interpreters merely because they were convenient during development.

### 2.2 Slint source policy: latest upstream default branch, then lock

Carry forward the preference for current upstream source, but **do not carry over the former framework's repository or assume that the branch is named `main`**. The authoritative Slint repository is `slint-ui/slint`; its default branch was displayed as `master` when this revision was prepared. Verify the actual default branch and current HEAD at implementation time. [S25]

At bootstrap, fetch that branch, inspect its manifests and examples, and resolve its current HEAD. Use Slint's runtime and build-time compiler from the **same upstream revision**, including any directly used framework companion crates. Do not substitute an unofficial repackaging or copy remembered APIs from an older tutorial.

Branch discovery command for an environment with network access:

```sh
git ls-remote --symref https://github.com/slint-ui/slint.git HEAD
```

The source-policy excerpt, assuming the upstream branch remains `master`, is:

```toml
[workspace.dependencies]
# Add the required production features in the application dependency declaration.
slint = { git = "https://github.com/slint-ui/slint", branch = "master", default-features = false }
slint-build = { git = "https://github.com/slint-ui/slint", branch = "master" }
```

This is not a complete compiling Cargo manifest: add the required compatibility, standard-library, windowing, accessibility, and renderer features after reading the fetched manifest. Record the actual selection; do not assume a feature name or compatibility flag survives every upstream update. [S2]

Commit `Cargo.lock`, record the resolved full SHA, branch, fetch date, toolchain, and feature graph in `docs/dependencies.md`, and use `--locked` in normal CI/release builds. An explicit identical `rev` for both crates is also acceptable after resolving the latest default-branch HEAD. Cargo's lockfile records Git dependency commits rather than automatically advancing them at every build. [S3]

Use reviewed, tested changes to advance the dependency. Do not delete the lockfile, fetch moving HEAD during every build, mix Git and registry copies of Slint, or claim an old checkout is latest. A later switch to an exact stable release is possible through a maintainer-approved ADR; it is not the default bootstrap policy. If fetching is unavailable, record the limitation and never invent a commit hash.

### 2.3 Backend, renderer, and feature selection

Start the video spike with the **Winit backend and a verified OpenGL renderer**, initially FemtoVG/OpenGL. This is an integration hypothesis, not a promise of the best renderer on every GPU. Evaluate another compatible renderer or a native video adapter when evidence warrants it. Slint distinguishes the OS/window backend from the renderer; renderer choice and video interoperability must be recorded separately. [S17]

Disable unnecessary default features explicitly. Enable accessibility and the compatibility feature required by the fetched revision; verify Linux Wayland/X11 coverage. Do not bundle Qt, multiple large rendering stacks, a tray integration, or live-preview machinery unless there is an actual product need. Do not disable accessibility or essential text support merely to make a benchmark look smaller. The upstream manifest is the authority for feature names. [S2]

Select and validate the runtime backend deliberately. A borrowed OpenGL texture cannot be assumed to work after an automatic switch to an unrelated graphics API. Display a useful capability error or use a separately validated adapter instead of silently introducing CPU frame copies. A software-rendered browsing fallback MAY exist, but it must not claim to support the optimized video gate.

Do not select a renderer solely because it supports dirty rectangles in another mode. Slint documents partial rendering for its software renderer; that is not a universal guarantee about its GPU renderers. Measure layout/binding work, draw work, and presentation independently. [S17]

### 2.4 Initial dependency choices

Use SQLite for local structured data, a Rust HTTP client with explicit network policy, and operating-system credential protection for account secrets. Prefer a small number of maintained dependencies. Verify APIs, enabled features, licenses, and build requirements rather than inventing crate names or methods.

Start with a thin, auditable libmpv binding or wrapper. Do not choose a wrapper merely because it hides frame copies. GStreamer is an alternative media backend to investigate through an ADR if the initial path is unsuitable; it is not a reason to ship two full media stacks by default. Slint is the UI choice, not a mandate to use a particular decoder or identical presentation plumbing on every OS.

## 3. Product scope and feature behavior

| Capability | Guest preview | Required for version 1 |
|---|---|---|
| Search videos, channels, and playlists | Yes | Paginated, cancellable, real results |
| Public VOD playback | Yes | Embedded video, audio, seeking, subtitles, quality selection |
| Channel pages and video details | Yes | Description, available metadata, paginated video lists |
| Local subscriptions and playlists | Yes | Persistent, editable, import/export |
| YouTube account connection | May be absent, clearly labeled | Verified account identity and reconnect/sign-out flow |
| Account subscriptions and playlists | No | Real remote reads; separate from local collections |
| Subscribe/unsubscribe, like/unlike, save/remove from playlist | Local actions only | Explicit authenticated actions with verified outcomes |
| Home page | Local feed or an honest empty state | Local feed by default; optional account-personalized home |
| Comments | May be deferred | Paginated read-only comments where supported |
| YouTube-served ad suppression | Yes, for supported playback paths | Tested; limitations disclosed |
| Local watch history | Optional and disabled initially | Opt-in, clearable, retention setting |
| Live streams, 4K, HDR, picture-in-picture | Experimental | Not required unless advertised as supported |
| Explicit downloads of public videos | Optional | Explicit per-video action, guest access only, visible progress/cancel/delete (Section 11.1) |

Never show synthetic content as live YouTube results. Fixture/demo mode must be labeled and must not be enabled in normal builds.

Local collections and YouTube-account collections must be visually and structurally distinct. Following a channel locally must not silently subscribe the account. Connecting an account must not upload existing local history, playlists, or subscriptions automatically.

Account writes require a deliberate user action. Disable the relevant control while its operation is pending or support explicit optimistic rollback. After a timeout, reconcile remote state before retrying a non-idempotent action. A toast saying “Saved” is not sufficient unless the remote operation actually succeeded.

### 3.1 Playback experience

Provide play/pause, scrubbing, volume/mute, elapsed/remaining time, fullscreen, quality selection, playback speed, subtitle selection, and a compact technical-info panel. Preserve volume and user settings. Preserve position across a recoverable stream refresh where possible.

Autoplay-next is off initially. Hovering thumbnails must not start video decoding or streaming. Selecting a video cancels superseded extraction requests. Only one video decoder/player should be active by default.

Errors must distinguish offline access, unavailable content, authentication needed, expired account session, rate limiting, unsupported format, extractor failure, and decoder/rendering failure. Show a useful recovery action rather than raw subprocess stderr.

## 4. Visual and interaction specification

### 4.1 Design direction

Create a clean, YouTube-inspired desktop experience, not a generic dashboard or developer tool. Use familiar information hierarchy and browsing patterns while retaining an independent application name and visual identity. Do not bundle copied YouTube logos, screenshots, or proprietary interface assets as application chrome.

Use restrained neutral surfaces, clear type, comfortable spacing, subtle borders, and a limited red accent. Avoid glassmorphism, oversized gradients, nested cards around everything, permanent animations, and decorative charts.

### 4.2 Layout

The top bar is approximately 56 logical pixels tall: navigation toggle and application identity on the left, a centered search field, and profile/settings controls on the right. The expanded sidebar is approximately 224–240 logical pixels; a compact icon rail is approximately 72 pixels. Main navigation includes Home, Subscriptions, Playlists, and History when enabled.

Use 16:9 thumbnails, approximately 10–12 pixel corner radii, a duration badge, two-line video titles, channel identity, and secondary metadata. The grid adapts to available width rather than stretching thumbnails arbitrarily. Start with 16–24 pixel gutters and a thumbnail-card minimum width around 260–300 pixels.

The watch page uses a dominant player and a right-hand related-video column on wide windows. Below approximately 1,200 logical pixels, move related videos below the primary content. Collapse navigation as needed. At small supported widths, prioritize the player, title, and essential controls rather than clipping the page.

Descriptions expand on demand. Comments and related results load incrementally. Use skeletons only while actual work is pending; stop their animation when the window is hidden or reduced motion is enabled.

### 4.3 Initial design tokens

| Token | Dark | Light |
|---|---|---|
| Window background | `#0F0F0F` | `#FFFFFF` |
| Raised surface | `#181818` | `#F5F5F5` |
| Primary text | `#F1F1F1` | `#0F0F0F` |
| Secondary text | `#AAAAAA` | `#606060` |
| Accent | `#FF0033` | `#D9002B` |
| Spacing scale | 4, 8, 12, 16, 24, 32 logical pixels | Same |

Treat these as starting tokens, not permission to ignore contrast. Validate foreground/background combinations, focus indicators, disabled states, and text scaling. Use system fonts by default. Support dark, light, and follow-system themes without unnecessary font downloads.

### 4.4 Input and accessibility

All essential flows must work by keyboard. Include visible focus, sensible focus restoration, accessible names, appropriate roles, and a logical reading order. Keep Slint's accessibility feature enabled, verify the fetched accessibility APIs and test actual screen-reader behavior on each supported platform rather than assuming framework support is sufficient.

Suggested playback shortcuts: Space/K play-pause, J/L seek backward/forward, arrows short seek or volume, M mute, F fullscreen, Escape exit fullscreen/close transient UI, and `/` focus search. Do not intercept these while editing text or using an input method. Support text selection/copy, IME composition, reduced motion, and high-DPI scaling.

### 4.5 Shared Slint component structure

Maintain one component library for the application shell, top search bar, sidebar, video card, virtualized feed, watch page, channel page, playlist page, account connection dialog, settings, and error/status controls. These are suggested component boundaries, not a requirement to generate empty files in advance.

Keep the long-lived video host separate from conditional controls and metadata sections. Showing controls, changing a like state, expanding a description, or updating elapsed time must not reconstruct the player or replace its graphics context. Keep the video rectangle opaque and rectangular by default; rounded thumbnail cards do not require rounded clipping of decoded video.

Do not put a global playback clock, pointer position, entire catalog, and account state into one frequently replaced UI model. Use narrowly scoped properties and models. Slint's property dependency tracking can localize updates, but the application must avoid unnecessary dependency fan-out. [S16]

## 5. Architecture and boundaries

Start with a small workspace. Module boundaries matter more than creating many empty crates.

```text
Shared compiled .slint components
                  |
Rust UI adapter: properties, row models, input, navigation
                  |
        Commands and domain events
                  |
       In-process Rust application core
       /          |           \
YouTube provider  Local store  Playback controller
       |                         |
Stream resolver           Media engine + presentation adapter
       |                         |
Supervised helpers        Validated GPU texture or native surface

Cross-cutting: network policy, credential vault, cancellation,
redaction, bounded caches, diagnostics, capability reporting.
```

Suggested ownership:

```text
crates/app/          Composition root, Slint bindings/models, input, navigation
crates/app/ui/       Shared .slint components, screens, theme tokens
crates/core/         Domain types, use cases, preferences, policy contracts
crates/youtube/      Catalog/account adapters and extractor supervisor
crates/media/        Player control, libmpv binding, platform presentation
crates/storage/      SQLite, migrations, caches, credential-vault adapter
xtask/              Optional build, fixture, and benchmark tooling
```

The core must not depend on Slint types. UI components and view-model adapters must not contain YouTube parsing, SQL, credential-handling logic, or raw media FFI. Media handles belong in the player/presentation boundary, not general domain events. Do not pass decoded frame byte arrays through the application command/event channel.

Define small internal interfaces such as `CatalogProvider`, `AccountProvider`, `StreamResolver`, `PlayerBackend`, `VideoPresenter`, and `CredentialVault`. These describe contracts to implement, not existing library APIs. Keep player policy separate from platform-specific graphics lifetimes without overengineering interchangeable implementations before they are needed.

### 5.1 Domain contracts

Use typed video, channel, playlist, and profile identifiers. Normalize provider responses into domain models before updating views. Preserve pagination cursors as opaque provider-owned data.

A resolved playback item must describe video/audio tracks, available subtitles, expiry information, required origin-specific headers, and the applicable network/session context. Stream URLs are short-lived secrets and must not be stored in the normal database or logs.

Every asynchronous operation carries a request identifier, cancellation mechanism, and profile/session generation. Discard results belonging to a previous search, account, or privacy mode. Sign-out must invalidate in-flight work before it can repopulate account caches.

Report provider capabilities independently: public browsing, account identity, subscriptions, playlist reads, playlist writes, likes, and authenticated playback. Failed support for one operation must not masquerade as successful support for all others.

### 5.2 Threading, scheduling, and bounded work

Run the Slint event loop and component access on the supported UI thread. Use the current documented event-loop handoff for worker results. Use weak component references in callbacks/tasks; capturing strong component handles in owned callbacks can create reference cycles. Verify the exact revision's threading/lifetime rules. [S1]

Keep network, extraction, image decoding, and blocking database work off the UI thread. A Rust async runtime may serve those operations, but it must have clear ownership, a bounded blocking pool, and no competing UI loop. Callbacks must not block waiting for a result that needs the UI or rendering context to progress.

Initial concurrency limits: four metadata requests, eight thumbnail requests, and one extraction job, with a tested maximum of two extractor jobs. Explicit downloads (Section 11.1) use their own pool of at most two supervised helper jobs so a long download never holds the playback extraction slot. Bound queues, response sizes, and retained results. Cancel irrelevant work and prioritize selected-video playback over speculative content.

Use coalescing or latest-value delivery for replaceable progress/frame-ready notifications. Do not use an unbounded event queue merely because an upstream example does. Preserve reliable delivery for terminal states, errors, and account-operation outcomes.

### 5.3 Reactive model and invalidation contract

Slint tracks property dependencies and reevaluates dirty bindings lazily. Its model API supports targeted row notifications and explicitly recommends updating a model rather than repeatedly replacing it. [S16], [S19]

For this application, the following are architectural requirements:

- Keep stable model instances. Apply row-level changes for thumbnail completion or a single subscription/playlist mutation; reserve model resets for an actual data-set replacement.
- Do not broadcast cursor coordinates into global state. Hover styling should depend on the relevant component's hover state, not on unrelated raw mouse-motion values.
- Do not notify a recommendations, search, or sidebar model because playback time or a video texture changed. Avoid rebuilding large Rust vectors and assigning them back to Slint for a one-row change.
- Keep property bindings cheap and free of I/O. Do not overwrite a declarative binding accidentally when setting a value from Rust; define ownership of each mutable property.

These rules localize application work. They do **not** promise that the GPU redraws only the changed rectangle or that the OS compositor performs no work on unchanged pixels. The implementation must measure those stages separately.

### 5.4 Separate clocks

Video presentation follows media timestamps and the media engine's update/timing mechanism. Application state must not be driven by a permanent video-frequency timer.

Default UI-update policy: change an elapsed-time string only when its displayed second changes; update a visible seek/buffer indicator at approximately 4 Hz initially. A smoother, locally scoped indicator is allowed after measurement. Scrubbing responds promptly to user input and coalesces redundant seek commands; it is not limited to the normal progress-update cadence.

Stop nonessential progress UI updates when controls and relevant labels are hidden, paused, or off-screen. State changes and explicit user actions must still be delivered immediately. Stop any local animation when it is no longer needed. A hidden video must not drive a continuous application redraw loop.

## 6. Video pipeline: mandatory feasibility gate

### 6.1 Verified building blocks and their limits

libmpv's public render interface supports OpenGL and software rendering. It supplies an update callback, but imposes context/threading restrictions and forbids calling its render functions from that callback. Slint supplies a rendering notifier and an API for borrowing an existing OpenGL texture. These are building blocks, not an already validated cross-platform player. [S5], [S4], [S20]

The initial hypothesis is: libmpv renders to an application-owned OpenGL framebuffer/texture compatible with the active Slint renderer, and Slint displays that texture without routine application-side CPU pixel readback. Inspect Slint's upstream OpenGL-texture example, but do not copy its animation scheduling without adapting it to an event-driven media player. [S21]

Slint's current GStreamer example enables its EGL-specific integration only on Linux; its alternative sink creates a CPU RGB pixel buffer. Copying that example is therefore not proof of efficient playback across all three desktop systems. [S22], [S23], [S24]

Do not invent an import API, assume that a GPU decoder produces a texture directly usable by Slint, or call an RGBA texture "zero-copy" without tracing the decoder-to-display path. Prove the implementation before building the complete product UI.

### 6.2 Required spike and evidence

Build a real Slint window containing moving video, working audio, and a representative static sidebar/recommendations layout. Start with a deterministic local clip, then a public YouTube video. Exercise pause, seek, subtitles, resizing, minimize/restore, fullscreen, end-of-file, failure recovery, and clean teardown.

Produce `docs/video-integration.md` with the exact application/Slint SHA, OS, GPU, driver, display settings, media-engine build, backend, renderer, graphics API, and decoder. Include commands, observations, frame-transfer stages, resource measurements, rejected approaches, and every unverified item. Include an initial `docs/platform-matrix.md` for macOS, Windows, Linux/X11, and Linux/Wayland.

A candidate path is supported only after runtime validation. An external mpv window is a diagnostic baseline, **not embedded playback**. A custom native child surface is allowed only when controls, clipping, overlays, focus, DPI, fullscreen, and teardown work inside the same application experience.

| Target | Initial direction to investigate | Evidence required before support is claimed |
|---|---|---|
| macOS | Slint OpenGL texture path first; a narrow native presentation adapter if needed | Actual hardware decode, context compatibility, transfer/power cost, resize/fullscreen lifecycle |
| Windows | Slint OpenGL texture path first; dedicated native GPU/surface interop if needed | Decoder-to-presentation interop, same/cross-adapter behavior, overlays, DPI and device loss |
| Linux/X11 | Compatible GL/decoder-buffer interop or a validated embedded surface | Actual decode backend, transfer/synchronization behavior and compositor interaction |
| Linux/Wayland | Compatible EGL/GL path or deliberately implemented subsurface integration | Native Wayland runtime evidence; do not treat XWayland or X11 embedding as that evidence |

This table defines research directions, not claims that these paths already work. A renderer or media-backend change may differ by platform while the `.slint` UI stays shared. Do not implement several speculative paths in parallel before proving the first one.

### 6.3 OpenGL texture ownership, lifetime, and synchronization

For the initial texture route, document the full path:

```text
Compressed packets -> active decoder -> decoded buffers
    -> any import/copy/color conversion -> libmpv render target
    -> Slint sampling/composition -> swap/present -> display
```

Classify each boundary as GPU-resident import, GPU copy/conversion, CPU map/readback, CPU conversion, CPU upload, or unknown. A GPU color conversion or intermediate GPU texture is not the same as a CPU readback; evaluate its measured cost. A single context-sharing step does not establish an end-to-end zero-copy pipeline.

Slint's borrowed-texture contract requires a valid compatible texture and the appropriate rendering context; borrowed images cannot simply be shared across separate Slint windows. Consult the exact API safety contract and preserve its requirements throughout resize, fullscreen, and destruction. [S4]

Keep presentation targets persistent across ordinary UI updates. Allocate/reallocate only when size/format/context changes require it; bound transient old/new targets during resize. If double buffering or fences are needed, define when each texture can be written, sampled, reused, and destroyed. Do not replace the entire player to resize one control.

Render only with the correct GL context current, and restore the graphics state required by Slint and libmpv. Document framebuffer format, orientation, physical-pixel sizing, synchronization, and color handling. Use the media engine's documented GL contract; do not assume arbitrary GUI and decoder contexts are interchangeable. [S26]

Tear down borrowed images and renderer references before deleting their textures/context. Recreate resources on context/device loss. Keep any cross-language `unsafe` or raw-handle code localized and document ownership/thread invariants. Future picture-in-picture requires its own validated window/context-sharing design, not reuse of an invalid borrowed handle.

### 6.4 Event-driven presentation and three kinds of rendering work

Use media update notifications to signal pending work and schedule the appropriate render opportunity. Coalesce wakeups; the callback itself must not perform UI mutation or rendering from an unauthorized thread. Do not put unconditional redraw requests in a rendering callback, and do not create a permanent 16 ms polling timer to detect frames. libmpv's callback/update contract must be followed at the selected revision. [S5]

Coordinate media timing and window presentation without busy waiting or blocking ordinary UI operations on the next video frame. Rendering notifications are opportunities to do valid graphics work, not evidence that a new video frame exists. Document how the chosen integration handles vsync, pause, expose/resize events, and hidden windows. Do not copy a demo loop that schedules its own next frame indefinitely.

Distinguish and instrument:

1. **Application work:** domain-to-UI assignments, model notifications, expensive derived-data construction, and binding/layout work where observable.
2. **GPU drawing:** UI/video render passes, draw submission, texture transfers and GPU time.
3. **Presentation:** swap/present cadence, compositor behavior, overlays where available, frame pacing and energy cost.

A changed video texture may legitimately cause a Slint window to draw again. That is not automatically a failure, provided unrelated application models/bindings are not being needlessly recomputed and the measured budgets pass. Conversely, property reactivity is not evidence that unchanged UI pixels were not redrawn. Document whole-window GPU work honestly and profile it before claiming an efficiency advantage.

Prefer independent video presentation when it materially reduces measured cost without breaking controls or shared UI. Do not make an independently composited OS layer an unverified blanket promise or demand a custom compositor merely to satisfy a slogan.

### 6.5 Hardware decoding and optimized-playback gate

Enable and verify hardware decoding at runtime. A requested option is not proof: mpv exposes `hwdec-current` for the active path. Record software fallback distinctly. [S6]

A configuration passes the **optimized playback gate** only when hardware decode is observed, the frame-transfer path is documented without routine full-frame CPU readback/conversion/re-upload, controls and synchronization work, and the applicable resource budgets pass. An unknown transfer stage cannot be presented as a verified zero-copy or low-copy stage.

Normal optimized playback must not use screenshot encoding, `glReadPixels`-style readback, per-frame CPU RGBA conversion, or a new `SharedPixelBuffer` populated with full-frame pixels each frame. A clearly labeled software diagnostic/compatibility path MAY exist; it does not satisfy the optimized gate and must not be silently selected under a hardware-performance claim.

Prefer a suitable integrated GPU when practical, but measure cross-GPU copies and power rather than forcing the wrong device. Select codecs using actual hardware support, requested quality, and an explicit efficiency preference; do not always choose AV1 or the maximum resolution. Offer lower hardware-decodable quality or a disclosed software fallback when necessary, without silently changing the requested quality to pass a test.

Validate SDR color range/matrix, aspect ratio, and subtitle composition. Do not advertise HDR merely because a decoder accepts a 10-bit format. Record active decoder, codec, resolution, frame rate, presenter, transfer classification, dropped frames, buffer size, and bandwidth in sanitized diagnostics.

### 6.6 Player lifecycle and network policy

Use one explicit player lifecycle: Idle, Resolving, Buffering, Playing, Paused, Seeking, Ended, and Failed. Maintain long-lived playback/presentation objects; UI control visibility and unrelated account changes must not recreate them. Account sign-out remains an explicit exception that stops authenticated playback.

Keep one active decoder/player by default. Handle device loss, display changes, and decoder reinitialization without leaking the old instance. Pause nonessential visual work while hidden; preserve correct engine timing and audio according to the user's playback policy rather than simply ignoring required callbacks.

Feed resolved media URLs to the player rather than loading entire videos into RAM. Support separate audio/video tracks. Disable uncontrolled user mpv configuration, scripts, and implicit extraction hooks so they cannot bypass network policy or introduce untracked processes.

The media engine makes its own requests. Enforce policy for manifests, segments, audio, subtitles, redirects, and retries; configuring the Rust HTTP client is not sufficient. Never apply account cookies globally to arbitrary CDN hosts. A narrow local network broker requires an ADR, explicit security boundaries, and inclusion in resource tests; it is not the default architecture.

## 7. YouTube provider and extraction

### 7.1 Separate catalog/account access from media extraction

Use yt-dlp initially for stream resolution and relevant metadata extraction. It is not a complete UI-ready implementation of personalized browsing or account mutations. Place catalog and account functionality in separately testable provider adapters.

A Rust adapter for the necessary unofficial YouTube/InnerTube operations is the intended initial direction. Research actual request/response behavior from current primary sources and sanitized fixtures; implement only the endpoints needed for the current milestone. YouTube.js is a protocol-reference candidate, not permission to silently replace the Rust application with a persistent JavaScript service. [S11]

Do not assume an unofficial provider is stable. Unknown response variants must yield structured partial results or explicit unsupported errors rather than panics. Optional fields must not break a whole page. Never invent successful account data when an operation is unavailable.

### 7.2 Supervised extraction

Invoke the extractor using an argument array, never shell interpolation. Use an explicit binary path and isolated configuration. Disable inherited user configuration and unapproved plugin discovery; yt-dlp documents configuration-disabling options that must be verified against the selected build. [S7]

Bound stdout/stderr, parse JSON defensively, set a timeout, kill and reap the entire owned process tree on cancellation, and redact diagnostic output. Suggested starting timeout: 45 seconds per extraction with at most two controlled retries where appropriate. Authentication failures, challenges, and rate limits are not reasons for an infinite retry loop.

Current yt-dlp YouTube support may require external JavaScript challenge tooling and a supported runtime. PO-token requirements also vary with the client and request. Package and test the exact combination rather than assuming one executable is sufficient forever. [S8], [S9]

Do not download and execute arbitrary code at playback time. Any runtime, scripts, or token provider must have a recorded source, integrity verification, license, permission model, and resource cost. No paid or third-party token server is a silent dependency. Unsupported extraction must fail transparently.

Track helper versions/build hashes separately from the application. Updates must be explicit and verifiable, with a compatibility check and rollback path. Do not auto-self-update the extractor outside the application's update policy.

### 7.3 Error recovery

Expire resolved URLs promptly. A recoverable expired URL may trigger one controlled re-resolution and resume attempt. Distinguish expired sessions, throttling, required proof tokens, region/access restrictions, and unsupported content rather than treating every HTTP 403 as the same error.

Honor backoff and server retry hints. Keep account requests conservative. Do not change identities, rotate accounts, or enable a public proxy automatically to conceal a failure.

## 8. Account connection and credential security

### 8.1 Product meaning of “login”

Version 1 must connect to a real YouTube session and verify the selected account/channel identity. It must support reconnecting an expired session and removing the connection. Reading a cookie file without a successful identity/account operation does not count as login.

The initial supported mechanism is **explicit, user-authorized browser-session import**, not a fake OAuth button. The yt-dlp documentation currently states that YouTube OAuth login no longer works for yt-dlp and recommends cookies; it also warns that account use can result in temporary or permanent bans. [S10]

Google's desktop OAuth flow exists for authorized Google API access, but that is distinct from obtaining a YouTube web session or working stream-extractor authentication. Ordinary browser sign-in does not hand session cookies to a native application's callback. Do not promise that OAuth alone solves the account/playback requirements. [S12]

### 8.2 Required connection flow

1. Explain what connection does: YouTube can associate authenticated activity with the account, session material is sensitive, unofficial-client compatibility is not guaranteed, and there is account risk.
2. Offer to open YouTube in the system browser. The user signs in on Google's real website, ideally in a dedicated browser profile. The application never asks for a Google password or a two-factor code.
3. Let the user explicitly select a locally exported cookie file. Provide current, verified documentation rather than silently installing an extension or scraping all browser profiles.
4. Validate size/format with explicit bounds (initially 4 MiB and 10,000 entries), filter to the minimum required YouTube session domains, discard unrelated cookies, and protect retained secrets. Cookie path, domain, secure, and expiry rules must be respected.
5. Perform a minimal account verification. Show the returned account/channel identity and allow explicit channel selection where supported. Report unsupported account configurations honestly.
6. Enable account features only after verification. Offer session refresh/reimport and clear local sign-out controls.

A later browser-profile import helper is optional and must require explicit profile selection and narrowly scoped consent. Do not scan browsers, copy entire cookie databases, bypass operating-system protections, or upload cookie material to a server. A seamless browser-extension/native-messaging bridge is future scope, not an undocumented requirement for version 1.

### 8.3 Secret handling

Treat session cookies as broad account credentials, not harmless preferences. Use OS credential storage for small secrets or a protected encryption key plus an authenticated-encrypted cookie store. Do not assume a keychain accepts arbitrarily large cookie jars.

When the secure store is unavailable, offer session-only in-memory operation. Never silently fall back to plaintext persistence. Apply restrictive file permissions and avoid secret-bearing temporary files; where an extractor requires one, use a private ephemeral directory, bounded lifetime, cleanup on failure, and documented crash cleanup. Do not promise forensic erasure from SSDs or process memory.

Credentials must not enter logs, telemetry, crash uploads, screenshots, command-line values, test fixtures, issue reports, or the normal SQLite database. Prefer a path to a protected temporary file or a verified secure IPC mechanism when crossing the extractor boundary; never put raw cookies in process arguments.

On sign-out: cancel authenticated operations, stop authenticated playback, invalidate the session generation, destroy stored credentials, clear account-specific caches, and preserve or delete local-only collections according to an explicit user choice. Explain that disconnecting this application does not necessarily revoke the user's browser session or sign them out of Google everywhere.

### 8.4 Account behavior and API-policy boundary

Account browsing, private data reads, and writes require separate tests and capability reporting. Personalized home must be optional. Do not deliberately submit extra watch-reporting or advertising beacons by default, but never promise that playback cannot affect Google's knowledge or account-side history.

The official YouTube API developer policies prohibit blocking advertisements and impose restrictions on player behavior. Therefore this specification does not assume the requested ad-suppressing client is approved for official API use. Do not ship shared OAuth credentials, borrow another application's identity, or introduce official API dependencies without a documented policy review. Separating modules or using cookies is not a legal exemption. [S13]

## 9. Advertisement suppression

YouTube-served ad suppression is enabled by default on supported paths. Render the application's own UI and play the resolved content stream without embedding YouTube's advertising player. Filter provider-designated advertising/promoted renderers out of browsing results. Do not create an ad-loading or ad-impression subsystem.

Do not equate this architecture with a guarantee that every current and future stream is ad-free. Advertisements can be represented differently across delivery paths. When advertising is inseparable or cannot be identified reliably, report the path as unsupported for guaranteed suppression rather than claiming success.

Do not rely on a broad DNS/domain blocklist as the entire implementation: the request policy must distinguish necessary media access from unwanted requests. Avoid blocking all video delivery to satisfy an “ad blocked” test. Do not fabricate ad-impression acknowledgements or invent a blocked-ad counter when no actual count is available.

Test ad handling separately for guest and connected-account playback. Include both known ad/promoted metadata fixtures and controlled live observations. A Premium account or a video that happened not to show an ad is not, by itself, evidence of ad suppression. Document exactly what was tested and distinguish YouTube-served ads from sponsorships already present inside a creator's video.

## 10. Privacy model and network policy

### 10.1 Honest threat model

The application aims to minimize its own collection and persistence, avoid browser-based tracking components, and keep local organization data under user control. It does **not** promise anonymity from YouTube when connecting directly, or account unlinkability while using account credentials.

Assume the operating system and current user account are trusted. Protect against accidental leakage, overbroad helper access, cross-profile data exposure, malicious remote metadata, and unnecessary third-party services. A compromised OS, invasive local administrator, or fully compromised media engine is outside the claimed protection boundary.

Display the difference between guest mode, connected-account mode, and any explicitly enabled proxy mode. “History disabled” means the application is not keeping its optional local watch history; it does not mean YouTube receives no viewing-related requests.

### 10.2 Defaults

No application account, first-party analytics, advertising identifiers, telemetry, automatic crash upload, cloud synchronization, remote fonts, or undisclosed third-party API calls. Update checking is manual initially; automatic checks require an explicit preference.

Start in guest mode with local watch/search history off, autoplay off, thumbnail previews off, and background subscription refresh off. Load remote thumbnails only when relevant content is requested and near the viewport. Do not prefetch entire catalogs.

A clean offline launch must open the shell and local library without a network request. Explain that ordinary thumbnails and metadata caches can still reveal browsing interests locally even when optional watch history is disabled. Provide a single clear-local-data action and configurable disk-cache limits.

### 10.3 Request policy

Centralize policy decisions, not necessarily every networking implementation. Classify metadata, media, images, subtitles, account operations, and update requests. Use explicit authentication attachment rules and re-check redirects. Isolate guest and authenticated cookies, client state, and caches.

Default to anonymous playback credentials for public content where the selected playback path permits it, even when account features are connected. Do not silently escalate a guest request to account-authenticated playback. Offer a clear action when authenticated access is necessary. This minimizes credential use; it does not guarantee unlinkability through IP addresses or other request context.

Never forward account session credentials to public metadata/proxy services. A user-configured transport tunnel with end-to-end HTTPS is a different trust boundary and must be documented as such.

Strict proxy support is optional for the first release. Before advertising it, verify metadata, extractor helpers, video/audio segments, subtitles, thumbnails, updates, redirects, and DNS behavior. Failure must stop the operation rather than silently fall back to direct access. A proxy checkbox that only affects the Rust HTTP client must not be labeled comprehensive protection.

### 10.4 Logging and diagnostics

Default logs contain operational events, structured error categories, and timing, but no cookies, tokens, signed URLs, account names, video titles, search queries, or browsing history. Sensitive types must have redacted formatting.

Diagnostic export is user-triggered and previewable. Include application/dependency versions, platform capabilities, timings, and sanitized failures. Raw packet captures, private provider responses, and memory dumps are not ordinary support attachments.

## 11. Local storage and caching

Use schema-versioned SQLite with transactional migrations, bounded queries, and a recoverable backup strategy. Keep blocking work off the UI thread. Do not load an entire user library into memory for rendering.

Store local subscriptions, playlists/items, preferences, opt-in history, and minimal cached metadata. Separate local data from account-derived data using profile identifiers. Store credentials only through the credential vault; the database may contain an opaque vault reference, never raw secrets.

Use an explicit cache namespace that includes provider, guest/account mode, account/channel identity where applicable, locale, and relevant policy version. Do not reuse personalized responses across profiles. Invalidate on sign-out and account switching.

Initial budgets: 32 MiB decoded thumbnail RAM, 64 MiB thumbnail GPU allocation where separately measurable, 256 MiB disk thumbnail cache, and bounded metadata caches. Budget duplicated CPU/GPU representations separately. Resize thumbnails for their display size before GPU upload and evict least-recently-used off-screen images.

Virtualize all long grids/lists. Retain only visible elements plus a small overscan area. Keep provider page caches bounded even after scrolling through thousands of items; persist or evict older pages instead of growing memory without limit.

For Slint, use a virtualized `ListView` of responsive thumbnail rows, or an equivalently measured virtualized component. The documented ListView instantiates visible items; this does not automatically bound the backing data model or image cache. Do not place every card in an unbounded `GridLayout` inside a scrolling container. [S18]

Keep row identity, focus, selection, and scroll position stable as pages arrive or a row updates. Recalculate column grouping only when available width changes the column count, not on playback progress. Use small bounded overscan. Cancelling off-screen thumbnail work must prevent stale completions from repopulating evicted views. Account for retained Slint image references and renderer-side texture caches when verifying that eviction actually releases memory.

Do not persist signed media URLs or complete authenticated provider responses. Playback buffering is transient, not a hidden downloader: the player never writes its stream buffer to disk, and only the explicit download feature in Section 11.1 stores media files. Start with a bounded forward buffer around 32–64 MiB for ordinary 1080p playback; larger formats may use an explicitly measured higher cap.

When local history is enabled, use a clear retention default such as 30 days and provide delete-all and per-item deletion. Deletion must also remove relevant search/index entries and derived local recommendations. Do not promise forensic removal of previously written filesystem blocks.

### 11.1 Explicit downloads

The application MAY save a public video for offline viewing when the user explicitly asks for that one video. When the feature is present it MUST follow these rules:

- **Explicit only.** Each download starts from a deliberate per-video command (for example a video card's context menu or the watch page). No background, speculative, scheduled, or automatic downloading; no bulk download of playlists, channels, subscriptions, or feeds; playback, hover, and prefetching never create download jobs.
- **Guest-accessible public content only.** Downloads use the anonymous guest extraction path. Never attach account cookies or session material. Private, members-only, purchased/rental, sign-in-required, DRM-protected, and in-progress live content is unsupported and fails with an explicit message; this is not permission to bypass any access, age, or purchase restriction.
- **Supervised helpers.** Use the same helper contract as stream resolution (Section 7.2): explicit binary paths, argument arrays, isolated configuration, bounded output, finite inactivity and overall timeouts, the shared rate-limit cooldown, and killing/reaping the owned process tree on cancellation, deletion, and application exit. A media merger such as FFmpeg is used only from an explicit, reviewed path. When it is unavailable, fall back to a single-file format and tell the user that quality is limited; never silently claim the requested quality.
- **Bounded concurrency.** At most two download jobs run at once; later requests wait in a visible queue. Download helpers are separate from the playback extraction slot, count toward whole-process-tree resource measurements, and do not run when no download is queued.
- **Quality.** Respect the user's default maximum-quality ceiling, and record the quality actually obtained.
- **App-owned storage.** Files live in a private `downloads` directory inside the application data directory. Keep a small, versioned index with atomic replacement that tolerates missing, externally deleted, or unindexed files. Do not store signed media URLs, cookies, or provider responses. Partial files are removed after cancellation or failure.
- **User control.** Show queued and running jobs with progress (and speed/remaining time when known) and a cancel action, and completed downloads with their metadata. Deletion requires confirmation and removes the media file, its artwork, and its index entry. Offer a way to reveal the file in the platform file manager.
- **Offline playback.** Playing a download uses the local-file playback path with its stored metadata and makes no YouTube request.

Downloaded files and their titles can reveal viewing interests. They are user files kept until the user deletes them; privacy documentation must state how they relate to the clear-local-data action.

## 12. Performance budgets and measurement

### 12.1 Meaning of the numbers

The following are **initial targets and release ceilings to validate**, not benchmark claims. The first measured prototype must establish a baseline. A missed target requires profiling; a missed ceiling blocks a performance-supported release on that test configuration unless the maintainer explicitly accepts a documented scope/budget change. The implementing agent must not silently raise limits.

Use release builds without debug logging. Record application commit, Slint SHA, backend/renderer selection, dependency hashes, OS, CPU, GPU, driver, display scale/refresh rate, codec, resolution, network conditions, and power mode.

Designate at least one hardware-decode-capable x86-64 laptop-class reference machine with an SSD and 16 GiB RAM for Windows/Linux work, and an Apple Silicon reference machine for macOS work when available. Record actual models; do not fabricate access to those machines. The required baseline is hardware-decodable 1080p60 SDR. A 4K60 claim requires a separate suitable hardware/codec test.

**CPU convention:** 100% means one fully utilized logical CPU, summed across all application-owned processes. **Memory convention:** include the application, extractor, JavaScript runtime, token provider, and any owned media helper. Report aggregate RSS/working-set measurements consistently within each platform, noting shared-page double counting. Also report Linux PSS, macOS footprint, or Windows private working set when available; these are different metrics, not interchangeable cross-platform numbers. Report GPU/unified-memory usage separately without pretending it is free or blindly adding overlapping counters.

### 12.2 Budgets

| Scenario or metric | Target | Release ceiling / acceptance condition |
|---|---:|---:|
| Settled library window, about 30 visible cached thumbnails | ≤180 MiB aggregate resident memory | ≤250 MiB |
| Same idle window, no work pending | ≤0.5% of one logical CPU | ≤1% over a 60-second sample |
| Minimized, not playing, no queued work | ≤0.1% of one logical CPU | ≤0.3%; no sustained render loop |
| Steady 1080p60 SDR hardware playback | ≤400 MiB aggregate resident memory | ≤650 MiB |
| Steady 1080p60 SDR hardware playback CPU | ≤25% of one logical CPU | ≤50% |
| Transient total during extraction/startup | ≤850 MiB aggregate resident memory | ≤1,200 MiB; helpers must return to zero when idle |
| 4K60 hardware playback, when claimed | ≤750 MiB and ≤60% of one logical CPU | ≤1,100 MiB and ≤100%; separate platform gate |
| Warm launch to interactive shell, without remote content | ≤1.5 seconds | ≤3 seconds |
| Input-to-feedback latency, excluding network | p95 ≤50 ms | p95 ≤100 ms |
| Scrolling cached content at 60 Hz | p95 frame time ≤16.7 ms | p99 ≤33.3 ms; no repeated long stalls |
| Steady playback dropped frames after warm-up | <0.1% | Must pass the fixed supported-format test |

For each memory/CPU scenario, collect at least 60 seconds after a documented warm-up; report mean, p95, and peak rather than one favorable snapshot. For transient jobs, record the complete process-tree peak including overlapping helpers. For startup and interaction latency, use repeated runs and report sample count and percentiles.

Measure network-controlled time-to-first-frame separately: request scheduling, metadata, extraction, connection, buffering, decode, and presentation. Do not make an Internet-wide playback-start guarantee or hide extraction latency by starting the clock afterward. Also measure first-frame latency for a deterministic local clip to isolate the player.

### 12.3 Mandatory profiling scenarios

Test idle, minimized idle, search, a 10,000-item fixture library, repeated navigation, 1080p playback, pause/resume, repeated seeking, account switching, and a 60-minute mixed-use soak. Run the input/rendering matrix below with a realistic cached sidebar and related-video list, not just an otherwise empty player window. Include codec fallback, missing hardware support, expired URLs, and cancellation during extraction.

After warm-up and cache saturation, repeated navigation/playback cycles must not show unbounded memory growth. Compare repeated identical checkpoints and investigate sustained increases greater than 10% rather than disguising leaks as cache growth.

Pause nonessential animations and background polling. No permanent 60/120 Hz UI redraw loop when nothing changes. Never keep an extractor or JavaScript runtime alive merely to make the next operation appear faster without accounting for that idle cost.

### 12.4 Input, invalidation, and playback profiling matrix

| Scenario | Required observation |
|---|---|
| Stationary pointer, settled browsing UI | No periodic application redraw requests, polling, model resets, or thumbnail work |
| Cursor moves across a noninteractive area | No app-wide property/model notifications or explicit redraw requests when visual state is unchanged; measure toolkit input and draw work separately |
| Cursor moves inside the same button/card | No repeated application model changes solely from raw pointer coordinates |
| Enter/leave a hover target | Correct local visual feedback; no unrelated list/model reconstruction |
| Video playing, controls hidden, metadata unchanged | Video advances; no playback-driven catalog/sidebar model notifications; hidden control timers stop |
| Video playing with controls visible | Only required progress values change; labels and indicators follow Section 5.4 rather than the decode frame rate |
| Video paused, stationary pointer | After settling and required expose events, no recurring frame requests or progress timers |
| Scrubbing, resize, fullscreen, subtitle change | Necessary updates remain responsive; no leaked render targets or player recreation from unrelated UI |
| Window minimized/occluded | No nonessential full-rate UI presentation; state/audio follow the declared playback policy |

In a deterministic no-background-data test, playback and irrelevant cursor movement must cause **zero unrelated catalog/sidebar model-change notifications and zero full-model resets**. This is a requirement about application-owned work, not a claim that no framework hit testing or compositor activity occurs. Log and investigate actual renderer behavior instead of hiding it.

Collect counters for UI assignments, model changes/resets, thumbnail decode/uploads, application redraw requests, media update callbacks, player renders, presents, and allocated media targets. Collect binding/layout or GPU traces where the selected tools expose them; label unavailable observations rather than inventing precision. Keep instrumentation local, sampled or disabled in normal release builds, and free of sensitive content.

### 12.5 Fair baselines and energy reporting

Compare the same licensed local clip in standalone mpv, a minimal browser video page when the format is supported, and the Slint prototype. A browser baseline is a test tool, not permission to ship a web frontend. Keep codec, resolution, frame rate, display dimensions/refresh/scale, hardware-decode state, subtitles, power mode, and playback duration equivalent. If an exact comparison is unavailable, state why rather than substitute an easier codec silently.

Report the application-shell overhead separately from the media engine: CPU, resident memory, graphics allocations, dropped frames, wakeups, and GPU/compositor cost where observable. Include whole owned process trees and note shared-memory accounting. For a browser baseline, state whether the figure covers the entire browser or an incremental tab; do not compare that ambiguously with the entire application.

Measure package/system energy or a documented power proxy when supported, and record method, idle baseline, display settings, thermals, and repeated-run variation. Do not claim "faster than web", "zero redraw", "zero-copy", or "best battery life" from a framework name, an FPS counter, or CPU percentage alone. No fixed superiority ratio is assumed by this specification.

## 13. Security, reliability, and testing

### 13.1 Automated coverage

Use unit tests for domain rules, cache partitioning, URL validation, preferences, account state transitions, and operation cancellation. Use sanitized fixture tests for provider parsing and ad/promoted renderer filtering. Unknown fields and missing optional fields must be covered.

Use integration tests for SQLite migrations, bounded caches, extractor timeout/cancellation, malformed/oversized output, secret redaction, account-session invalidation, and signed-URL refresh. Test the player against small licensed/local fixtures covering audio/video synchronization, subtitles, seek, end-of-file, and failure recovery.

Fuzz or property-test cookie import, provider JSON parsing, external URL handling, and untrusted metadata. Treat descriptions/titles as untrusted text, not executable HTML. Reject unsafe external-link schemes and credential-bearing arbitrary URLs. Restrict media protocols and redirect destinations so malicious metadata cannot trigger local-file reads or unintended local-network access.

Slint interaction tests should cover search focus, navigation, loading/empty/error states, virtualized scrolling, player controls, and account connection/disconnection. Add screenshot regression checks with deterministic fixtures, but do not mistake a screenshot test for a video playback test. Test incremental model updates, weak-reference teardown, column-count changes, pause/minimize scheduling, and render-target cleanup. Run compile/lint tests for deliberate production feature combinations; do not enable every mutually incompatible graphics feature merely to run an indiscriminate all-features build.

### 13.2 Live tests and account safety

Keep routine CI deterministic and network-independent. Live YouTube smoke tests are opt-in, low-rate, and separate from pull-request tests. Never put real cookies or production accounts in public CI or fixtures. Account validation may require a human-run local test; record that requirement honestly rather than requesting credentials in chat.

Manual platform tests cover accessibility, IME, DPI changes, keyboard focus, fullscreen, display changes, Wayland/X11 differences, hardware decoding, and process cleanup. Headless compilation does not satisfy these gates.

### 13.3 Release acceptance cases

| ID | Required observable result |
|---|---|
| AC-01 | Slint runtime/build compiler resolve coherently from the verified upstream default-branch HEAD at adoption; SHA, feature selection, and lockfile are recorded |
| AC-02 | Clean offline launch displays an interactive native shell without remote requests |
| AC-03 | Real search returns paginated results; stale queries cannot overwrite a newer query |
| AC-04 | A real public video plays inside the Slint application with synchronized audio and working controls |
| AC-05 | Hardware decoding and presentation behavior are measured, not inferred from a requested option |
| AC-06 | Large fixture libraries remain virtualized with bounded image/page caches |
| AC-07 | Supported ad/promoted items are excluded; live ad-suppression limitations are documented |
| AC-08 | Explicit session import verifies account identity and reads real account subscriptions/playlists |
| AC-09 | Authenticated subscribe/like/save operations succeed remotely or report a real error and reconcile state |
| AC-10 | Sign-out cancels operations, stops authenticated playback, and prevents stale account-data reappearance |
| AC-11 | Secret scans find no sensitive values in logs, process arguments, exports, fixtures, or the normal database |
| AC-12 | All applicable resource ceilings pass on the declared supported configurations |
| AC-13 | Offline, expired-session, throttling, unsupported-format, and extraction failures are recoverable or explained |
| AC-14 | All owned helper processes terminate on cancellation and application exit |
| AC-15 | Strict proxy mode, if advertised, passes a whole-process-tree egress and failure test |
| AC-16 | The documented source build and packaged application work without undeclared local dependencies |
| AC-17 | All application screens and controls use the shared compiled Slint UI; OS-specific presentation is isolated |
| AC-18 | Cursor/hover/playback tests show no unrelated catalog/sidebar model updates or full resets |
| AC-19 | Paused and settled hidden/idle states have no self-sustaining app redraw/progress loop |
| AC-20 | Every performance-supported platform passes the hardware/transfer-path gate without routine CPU frame copies |
| AC-21 | Decoder, transfer stages, renderer work, and baseline comparisons are documented; unavailable evidence is explicit |
| AC-22 | Texture/context lifecycle survives resize, fullscreen, repeated playback, and teardown without leaks or stale handles |
| AC-23 | The selected Slint/framework/media license route and redistribution obligations are recorded before release |
| AC-24 | If downloads are offered: only explicit guest downloads run, at most two at once; cancellation and exit terminate their helpers and remove partial files; deletion removes file and index entry; downloads play offline through the local-file path |

No release may claim an untested operating system, hardware decoder, account feature, or proxy guarantee as supported.

## 14. Delivery sequence and agent working method

### M0 — Feasibility and reproducible foundation

Inspect the existing repository and preserve unrelated work. Resolve Slint's actual default-branch HEAD, align runtime/compiler dependencies, compile a minimal `.slint` UI, and run a native window. Verify the active window backend, renderer, and graphics API rather than assuming selection from Cargo features.

Implement local embedded playback with libmpv and the chosen presenter. Add representative static browsing UI, counters, and the cursor/paused/active-playback tests before extensive visual work. Profile texture transfers, actual hardware decode, frame pacing, memory, and CPU; compare a standalone mpv baseline. Write the integration ADR and capability matrix. Research account/session-import feasibility in parallel so it is not discovered only after UI polish.

**Functional exit gate:** reproducible native Slint window, real embedded video/audio and controls, coherent dependency revision, known active rendering stack, and evidence for completed tests. A static mockup or external player alone does not pass.

**Optimized exit gate:** documented hardware decode and transfer path without routine CPU readback/re-upload, bounded queues/resources, correct pause/hidden behavior, unrelated-model-update tests passing, and initial budgets measured. If this gate is blocked, continue useful independent work but do not claim an optimized player or proceed as though the issue is solved.

### M1 — Real guest playback slice

Implement URL handling, a supervised resolver, real public playback, controls, error states, and search. Disable inherited player/extractor configuration. Test cancellation, stream expiry, shutdown, and the initial ad-suppression path.

**Exit gate:** a user can find a real video and watch it without hidden helper dependencies or fabricated data.

### M2 — Familiar UI and local library

Implement the responsive YouTube-inspired shell as shared `.slint` components, virtualized thumbnail rows/grids, watch page, channel pages, local subscriptions/playlists, themes, keyboard behavior, thumbnails, and bounded storage. Use incremental row updates and stable models; the player host must survive ordinary view-state changes. Add deterministic UI fixtures and accessibility checks.

**Exit gate:** coherent end-to-end browsing with no placeholder buttons on advertised features and an idle resource baseline inside its ceiling.

### M3 — Real account functionality

Implement protected session import, verified identity/channel selection, authenticated provider reads, explicit mutations, session expiry, cache isolation, and sign-out. Validate account risks and privacy disclosures. Human-only integration steps must be documented.

**Exit gate:** account acceptance cases pass with an explicitly authorized local test account; absence of such a test remains a recorded release blocker, not fabricated completion.

### M4 — Hardening and first supported release

Profile and reduce resource usage, repeat the input/invalidation and fair playback baselines, complete privacy/security tests, review dependency licensing, test clean installation, publish build instructions, and complete the supported-platform acceptance matrix.

**Exit gate:** all mandatory version-1 capabilities and applicable release ceilings pass on at least one declared platform.

### M5 — Additional platform qualification

Reuse the same `.slint` UI and implement/validate only the remaining video presentation adapters and platform integrations. Run the same acceptance suite separately for every newly supported target. Do not carry assumptions from the first platform into release claims for another.

### Working rules

Implement small working vertical slices, not a large forest of placeholders. Keep the workspace buildable after each meaningful change. Preserve existing user work. Use ADRs for consequential changes and update a short `docs/progress.md` with evidence, commands, failures, and the next concrete task. Keep `docs/dependencies.md`, `docs/video-integration.md`, `docs/performance.md`, and `docs/platform-matrix.md` consistent with the actual implementation; record selected license routes in `docs/licensing.md`. Do not fill these documents with invented measurements or undocumented API promises.

The agent must actually execute checks when its environment permits. When a GPU, operating system, network, or credential is unavailable, it must state the exact unverified item and continue useful independent work. It must never invent command output, performance numbers, successful login, screenshots of an unrun build, or test passes.

## 15. Open-source and distribution requirements

Proposed default license for newly authored application code: **GPL-3.0-or-later**, subject to a recorded maintainer decision and an audit of the exact distributed dependencies. Preserve existing file ownership and licensing; do not relicense unrelated code automatically.

For this open-source project, the intended Slint route is **its GPLv3 option**, not an assumed MIT license. Slint separately licenses its framework, examples, and documentation; MIT-licensed examples do not make the linked framework MIT. Its royalty-free and commercial alternatives must not be silently selected or confused with the chosen open-source distribution route. Record exact SPDX identifiers and the combined-distribution obligations for the fetched files/build. [S15]

An original file marked GPL-3.0-or-later does not change a dependency's GPL-3.0-only grant or automatically make the entire bundle redistributable under every later GPL version. Resolve the actual combined distribution terms in `docs/licensing.md` before publishing binaries. This is a review requirement, not a license-compatibility assurance.

mpv documents a GPL-default build and conditions for LGPL configurations; audit the actual build and linked codecs. Include Slint renderer dependencies, FFmpeg, extractor/runtime bundles, fonts/icons, and any optional GStreamer plugins in that review. [S14]

Include the actual license text, third-party notices, a dependency/build inventory, and a software bill of materials. Document native build flags and redistribution obligations. Do not assume dynamic linking or downloading a dependency later automatically resolves licensing concerns.

Provide `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, a code of conduct, build instructions, architecture notes, privacy documentation, troubleshooting, and an explicit platform support matrix. Make a source build possible without project-owned API secrets or paid service credentials; document native toolchain prerequisites separately from signed/notarized official binaries.

Package only verified helper binaries/scripts. Verify integrity, prevent archive path traversal during updates, and reject untrusted update manifests. Never execute a remote shell install command from inside the application. App and helper updates must have explicit trust roots, compatibility checks, and a rollback story; delegated OS/package-manager updates are acceptable when documented.

Before public distribution, review applicable platform terms, account risks, trademark/branding, open-source obligations, and packaging requirements. Open source is not an exemption from those issues. This specification is an engineering proposal, not a legal assurance of approval or compliance.

## 16. Principal risks and required responses

| Risk | Required response |
|---|---|
| Slint upstream or a renderer/interop API changes | Keep runtime/compiler revisions aligned, inspect source, lock dependencies, isolate graphics glue, record minimal patches |
| GPU integration needs substantial platform work | Prove one path first; keep a shared UI and honest experimental support status elsewhere |
| Fine-grained reactivity is mistaken for partial GPU redraw | Instrument application, renderer, and presentation stages independently |
| A sample player uses hidden CPU frame copies | Trace the complete path; a CPU-copy fallback does not pass the optimized gate |
| OpenGL integration is efficient on only some targets | Evaluate another renderer/presenter through an ADR; do not silently change performance claims |
| Slint licensing is mistaken for the examples' MIT license | Record the framework GPL route and review the exact combined distribution |
| YouTube changes extraction, authentication, or ads | Isolate adapters, maintain sanitized fixtures, use controlled updates, expose accurate capability failures |
| Account credentials leak or activity becomes more identifiable | Minimize credential use, protect storage, isolate caches, redact logs, explain account mode |
| Helpers erase expected memory/CPU savings | Measure the whole process tree, stop idle helpers, enforce resource gates |
| API/terms/licensing assumptions block distribution | Review before shipping; do not claim approval or hide the selected integration method |

## Sources and verification notes

Primary-source references checked for this revision on September 28, 2026. Upstream branches and published `latest` documentation can differ; the implementation must inspect the exact fetched commit before relying on an API. Source review is not runtime validation. This document records no fetched application dependency SHA and claims no successful compilation, video integration, account test, or benchmark.

The product requirements, budgets, layouts, and proposed architecture are design decisions. Citations support the underlying library/platform constraints, not a prediction that the completed application will meet the budgets. Re-check account and API-policy guidance before implementation and release.

- **[S1]** Slint Rust integration: compiled components, event loop, and handle lifetimes.
- **[S2]** Slint upstream Rust manifest and feature declarations.
- **[S3]** Cargo dependency selection and Git lockfile behavior.
- **[S4]** Slint borrowed OpenGL texture API and safety contract.
- **[S5]** libmpv public render API, supported backends, callbacks, and threading.
- **[S6]** mpv manual, including embedding and actual decoder diagnostics.
- **[S7]** yt-dlp repository and CLI documentation.
- **[S8]** yt-dlp external JavaScript support.
- **[S9]** yt-dlp PO-token guidance.
- **[S10]** yt-dlp YouTube authentication and cookie guidance.
- **[S11]** YouTube.js / InnerTube reference implementation.
- **[S12]** Google native-app OAuth documentation.
- **[S13]** YouTube API developer policies.
- **[S14]** mpv build/licensing information.
- **[S15]** Slint framework, examples, and documentation license declarations.
- **[S16]** Slint property reactivity, lazy evaluation, and overwritten bindings.
- **[S17]** Slint backend/renderer distinctions and rendering capabilities.
- **[S18]** Slint ListView virtualization.
- **[S19]** Slint Model and incremental row-notification contract.
- **[S20]** Slint Window rendering notifier and redraw interface.
- **[S21]** Slint upstream OpenGL texture example.
- **[S22]** Slint GStreamer example: platform selection for EGL integration.
- **[S23]** Slint GStreamer example: EGL/texture integration.
- **[S24]** Slint GStreamer example: CPU RGB pixel-buffer path.
- **[S25]** Slint authoritative repository and displayed default branch.
- **[S26]** libmpv OpenGL render contract.

[S1]: https://docs.slint.dev/latest/docs/rust/slint/
[S2]: https://raw.githubusercontent.com/slint-ui/slint/master/api/rs/slint/Cargo.toml
[S3]: https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html
[S4]: https://docs.slint.dev/latest/docs/rust/slint/struct.BorrowedOpenGLTextureBuilder.html
[S5]: https://raw.githubusercontent.com/mpv-player/mpv/master/include/mpv/render.h
[S6]: https://mpv.io/manual/stable/
[S7]: https://github.com/yt-dlp/yt-dlp
[S8]: https://github.com/yt-dlp/yt-dlp/wiki/EJS
[S9]: https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide
[S10]: https://github.com/yt-dlp/yt-dlp/wiki/Extractors
[S11]: https://github.com/LuanRT/YouTube.js
[S12]: https://developers.google.com/identity/protocols/oauth2/native-app
[S13]: https://developers.google.com/youtube/terms/developer-policies
[S14]: https://raw.githubusercontent.com/mpv-player/mpv/master/Copyright
[S15]: https://raw.githubusercontent.com/slint-ui/slint/master/LICENSE.md
[S16]: https://docs.slint.dev/latest/docs/slint/guide/language/concepts/reactivity/
[S17]: https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/
[S18]: https://docs.slint.dev/latest/docs/slint/reference/std-widgets/views/listview/
[S19]: https://docs.slint.dev/latest/docs/rust/slint/trait.Model.html
[S20]: https://docs.slint.dev/latest/docs/rust/slint/struct.Window.html
[S21]: https://raw.githubusercontent.com/slint-ui/slint/master/examples/opengl_texture/main.rs
[S22]: https://raw.githubusercontent.com/slint-ui/slint/master/examples/gstreamer-player/build.rs
[S23]: https://raw.githubusercontent.com/slint-ui/slint/master/examples/gstreamer-player/slint_video_sink/egl_integration.rs
[S24]: https://raw.githubusercontent.com/slint-ui/slint/master/examples/gstreamer-player/slint_video_sink/software_rendering.rs
[S25]: https://github.com/slint-ui/slint
[S26]: https://raw.githubusercontent.com/mpv-player/mpv/master/include/mpv/render_gl.h
