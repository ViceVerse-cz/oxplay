# Contributing

Read [SPEC.md](SPEC.md) before changing architecture. Its MUST requirements and
acceptance cases are the product contract. See [docs/progress.md](docs/progress.md)
for implemented behavior and remaining gates, and
[docs/licensing.md](docs/licensing.md) for dependency obligations.

Keep the Rust core in process and all application screens/controls in shared,
compiled Slint components. Keep credentials, provider JSON, graphics handles,
SQL and Slint types out of domain objects. Prefer one tested vertical slice over
empty abstractions. Performance targets may not be silently weakened, and a
software decode/copy fallback must never be described as optimized playback.

Use the exact Slint revision in the workspace and committed lockfile. Deliberate
updates must keep runtime/compiler/companions coherent and record the reviewed
source and feature selection. Once dependencies are resolved, normal commands
are:

```sh
cargo fmt --all --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --locked -- -D warnings
```

Run the documented target-specific media/build checks too. Do not use an
indiscriminate all-features build for mutually incompatible rendering stacks.
Tests should prove domain/ownership/cancellation behavior or acceptance gates,
not repeat implementation details. Keep routine CI offline and deterministic;
label fixtures/demo mode explicitly.

The debug test suite includes [shared control regressions](docs/controlled-widgets.md)
using the pinned Slint mock backend. It renders no pixels and uses synthetic
acknowledgements; native media and platform checks remain separate. Release
builds omit the element metadata and this development-only test target.

Keep changes focused and preserve existing ownership. Explain the user-visible
problem, resulting behavior, validation and remaining limits in a pull request.
Record commands and actual results; compilation is not runtime platform,
accessibility, decoder, account or performance validation. Attach only sanitized
evidence and keep integration/performance/platform documents consistent.

Never submit cookies, auth codes, passwords, signed URLs, private provider JSON,
or raw crash/memory dumps. A human must explicitly authorize local account
tests and each live mutation; possession of credentials grants no permission.
See [SECURITY.md](SECURITY.md). Do not bypass access challenges or content
authorization restrictions.

New original contributions are accepted under GPL-3.0-or-later, without
copyright assignment. By contributing, confirm you have the right to submit
the work under those terms. Identify copied third-party material and preserve
its notices/licenses; this statement does not relicense dependencies or the
pre-existing specification. Be respectful under the
[code of conduct](CODE_OF_CONDUCT.md).
