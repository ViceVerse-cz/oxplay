# Shared UI accessibility qualification

Slint's accessibility feature remains enabled at the adopted source revision
`cf3b07d4917e6759a63b0c03913a2594ec653414`. This is implementation evidence,
not a screen-reader acceptance pass.

The shared `Action` component supplies a button role, accessible name, enabled
state, default action, keyboard activation and visible focus. Compact icon-only
buttons retain their text names. Active navigation/library/detail sections expose
selection; the playback-details toggle exposes its checked state. Pure visual
emphasis on the import action is kept separate from selection semantics. Video cards expose their kind/title/channel and
keyboard/default actions. Official Lucide SVGs are visual assets; their appearance
is not used as the only accessible name.

The 2026-09-29 audit found and added persistent names for the seek and volume
sliders, search and collection-name fields, caption selector, result/channel
filters, local collection selectors, playback speed/quality, appearance and
history retention. Previous and next collection pages now have distinct
accessible names even though their visible labels remain compact. Slider units
are identified as seconds and percent. The pinned Fluent widgets already expose
slider value/range/actions and combo-box value/expanded state; the app preserves
those implementations rather than replacing them with custom controls.

The pinned `internal/core/window.rs` saves a weak reference to the prior focused
item when opening a popup, restores it on close, and handles Escape dismissal.
This source behavior is not a runtime focus-restoration result. The earlier
native lifecycle diagnostic exercised playback keyboard shortcuts and the rule
that editable search text consumes its own input first. Full keyboard traversal,
virtualized row focus/scroll stability, popup focus restoration, IME composition,
text scaling and reduced-motion checks remain open.

On this host, the direct ApplicationServices probe:

```sh
/usr/bin/swift -e 'import ApplicationServices; print("AXIsProcessTrusted=\(AXIsProcessTrusted())")'
```

returned `AXIsProcessTrusted=false`. Accessibility Inspector and VoiceOver are
installed, but this process lacks accessibility-inspection permission. No privacy
settings were changed or VoiceOver enabled as part of the probe. Actual VoiceOver
roles/names/actions/reading order and IME behavior need authorized local testing;
Linux/X11, native Wayland and Windows need their own runtime qualification.
