# Compressed media range transport

## Optional range timing

`SEREIN_HTTP_TIMING=1` enables fixed-size per-source aggregates. One summary is
printed when the last source clone, reader, and active measurement release the
shared allocation. No URL, host, header, offset, source identifier, or error
message enters the diagnostic structure or output. Read-triggered and
size-triggered fetches separately count starts, completions (including errors),
errors, cancellations, validated bytes, cumulative duration and maximum duration.
The first completion delay starts at that source's first fetch, not extraction or
UI startup. All durations use monotonic microseconds.

The interval covers lazy runtime/client setup, DNS, redirects, HTTP body receipt,
and range/container validation together. It does not isolate those stages. Bytes
count successfully validated ranges, including refetches; partial/error response
bytes and transport overhead are excluded. Cancellation before a fetch begins is
not a fetch and is not counted. An unwound unfinished attempt is counted as an
error. These lifetime totals cannot themselves identify when FILE_LOADED occurred.

When disabled, fetching adds only an option check: no diagnostic clock reads,
counter updates, locking or allocations. The existing20-second fetch deadline,
range size, cancellation and representation-identity checks are unchanged.
Three deterministic tests cover separate trigger accounting, redacted output,
unfinished attempts, cancelled fetches without I/O, and final-reader ownership.
The current network suite passes22 unit tests and three helper integration tests,
with one explicit legacy DNS smoke ignored; strict Clippy passes. A release
scoped-playback run collected the live timing totals recorded below. Those
totals precede DNS helper isolation and do not qualify the new helper's cost.

This is an implementation in progress, not a whole-process network-policy pass.
The direct libmpv path remains separately identified. Authenticated playback is
not enabled by this work.

`serein-network` owns a guest-only HTTPS range reader with an allocation-only
open and a local, nonblocking seek operation. Its lazy current-thread HTTP runtime
and socket/body work run on the media demux thread. On macOS, native DNS runs in
the bounded helper described below; other targets retain their unqualified
in-process resolver. Cancellation has an
independent atomic/notification handle; it can interrupt a pending response
without acquiring the reader lock. No signed URL or response text appears in its
public errors or debug formatting.

Each requested range is at most 1 MiB, and reads copy at most 64 KiB of compressed
data to the media engine. One previous range may coexist with its replacement
while a request completes, so range payload storage peaks at 2 MiB per reader;
HTTP/TLS/DNS allocations and libmpv's demux cache are additional, not hidden in
that number. The media boundary must separately cap active readers. Reads reject
wrong offsets, inconsistent totals/validators, truncation, excess bytes and
unsupported status/encoding. A read failure is not EOF. MP4 `ftyp` or WebM/EBML
signatures are required before data enters the demuxer; this is not by itself a
proof that every container-internal reference is confined.

Production addresses must be HTTPS googlevideo subdomains without credentials,
custom ports or fragments. DNS answers must contain only ordinary public
unicast addresses; the filtered answers are the connector's actual inputs.
System DNS configuration, not a substitute public DNS service, selects upstream
resolvers. Redirects are explicit and limited to three; each destination is
revalidated. Extractor headers apply only to their exact initial origin and are
removed across an origin change. If-Range validators are scoped to the exact
final representation URL and are never forwarded to redirect intermediaries. This initial API rejects Cookie, Authorization,
Host, Range and other unreviewed fields. Proxy inheritance and automatic retries
are disabled. TLS verification uses Rustls defaults and is never disabled.

The whole request, including redirects/body consumption, has a 20-second async
deadline; connect timeout is 8 seconds. There is no runtime UI polling loop or
HTTP service. Native synchronous initialization is qualified below.

On macOS, DNS uses the system `DNSServiceGetAddrInfo` API through libSystem,
with separate concurrent A and AAAA queries, interface zero and no resolver
override. The helper uses that same system API without parsing nameserver
strings; its process identity differs from the application, so per-app VPN and
scoped routing remain unqualified. Hickory's system-config parser rejected this host's
scoped IPv6 nameserver (`%en0`); stripping that scope or silently selecting other
nameservers would change routing. Native responses are bounded to 32 addresses
per family, and any nonpublic answer rejects the entire result. One family's
`NoSuchRecord` does not discard the other family's valid answer. No application
DNS cache is added on macOS; the system daemon controls its own cache.

Both native queries share a three-second async deadline. Their uniquely owned
references use Tokio `AsyncFd` readiness and nonblocking descriptors; only
`DNSServiceProcessResult` reads the IPC socket. Cancellation/timeout drops the
readiness registration before deallocating the native reference and finally its
callback state. FFI signatures, callback error rules, and this teardown order
were checked against the installed macOS 27 SDK `dns_sd.h`; libSystem exports
were checked in its SDK stub. The native resolver/socket-drop smoke passed on
this host, including observing `EBADF` after cancellation cleanup.

Before the helper isolation described below, the deadline did **not** establish
a hard bound on synchronous `DNSServiceGetAddrInfo` initialization IPC. It ran
on the media demux thread, never the UI thread. Apple's source at verified upstream HEAD
`d4658af3f5f291311c6aee4210aa6d39bda82bbe` includes a 60-second `select`
safeguard before receiving initialization status. This is not an overall call
deadline: connect/send have no explicit timeout there, the safeguard skips
descriptors at or above `FD_SETSIZE`, and the following blocking read loop has
no absolute deadline. Our nonblocking mode is set only after initialization.
The same source shows incomplete nonblocking response-frame handling; a partial
frame can fail processing. Errors fail closed without a resolver fallback.
The installed libSystem build has **not** been associated with this exact source
revision. Native daemon stall/partial-frame fault injection and scoped-VPN
qualification remain pending. See the [pinned Apple DNS-SD client source](https://github.com/apple-oss-distributions/mDNSResponder/blob/d4658af3f5f291311c6aee4210aa6d39bda82bbe/mDNSShared/dnssd_clientstub.c).

Other targets currently select Hickory 0.26.3 using system configuration, a
three-second timeout, one configured attempt and a cache cap of 32 responses per
reader runtime. Those targets have not been built or validated here.

Before helper isolation, fifteen deterministic tests passed, including local HTTP range
responses, origin-changing redirects with header assertions, inconsistent range/
ETag rejection, cancellation while the server withholds a response, offline
initial open/seek, public-address filtering, header bounds and native DNS callback
handling. Strict Clippy and the separately invoked native DNS smoke passed. The
loopback policy used by fixtures is private to tests. Subsequent native libmpv
observations are recorded below; broader CDN qualification, resource comparisons,
DNS failure/redirect-chain tests and clean-machine qualification remain pending.

An explicit guest diagnostic against public video `aqz-KE-bpKQ` subsequently
passed for both separately resolved video and audio tracks: initial 65,536-byte
read, seek to the last 4,096 bytes, seek back and another 65,536-byte read, then
permanent cancellation. This used strict HTTP 206/range validation, native
system DNS and Rustls verification. The earlier invocation returned the generic
`Unsupported` error; its HTTP status/container cause was not captured, and the
successful rerun is not attributed to an invented CDN fix. The later structured
status error reports only a numeric code. Signed addresses, response bodies and
full media content were not retained. This exercises the reader directly, not
native libmpv playback or whole-process egress qualification.

```sh
cargo clippy --locked -p serein-network --all-targets -- -D warnings
cargo test --locked -p serein-network native_resolution_and_cancellation_close_the_owned_socket -- --ignored --nocapture
cargo run --locked -p serein-network --example public_range -- 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
```

## Embedded native check

A 60-second macOS debug run of `--scoped-media --url` against the same public
video exited successfully. Binary SHA-256: `bedd6b66e75fb1cfd7704cb1b08ad3dc3138be880ea2cad7b2e09d76d8b1ce68`. At 5 and 10 seconds it remained Buffering; by 25 seconds it
reported embedded VideoToolbox H.264 1920×1080 at 60 fps and Opus through
AVFoundation. The final media position was 48.017 seconds, with no engine error,
two persistent render-target allocations and zero catalog changes/resets.
[Raw sanitized evidence](evidence/2026-09-29-scoped-media-native.log).

This is a functional check only. It was a debug build with substantial competing
Rust compilation and an explicit stack sample; 1,469 VO drops were recorded.
It does not pass smoothness, resource, energy or perceptual A/V synchronization
gates. The initial combined expiry diagnostic had failed its 12-second readiness
assertion while still buffering. The later stack sample found playback already
active and cannot explain initial latency. The later release check below records bounded opt-in range timings; the
debug startup delay is not declared fixed by waiting longer.

```sh
target/debug/serein --scoped-media --url 'https://www.youtube.com/watch?v=aqz-KE-bpKQ' --ui-size 1100x760 --diagnostics --quit-after 60 --data-root /ABSOLUTE/ISOLATED/ROOT
```


## Release transport timings

At commit `b1f4074defcb36371c7bc8cd25966474d8e6d0d5`, release binary SHA-256
`62e90c91af4d382b1d6e3cf942660977e627f1f9a52d6274b0a0a595b71a6c8a`
ran the same public guest video with `SEREIN_HTTP_TIMING=1`, `--scoped-media`,
`--diagnostics`, and `--quit-after 60`. It exited successfully. The five-second
checkpoint observed VideoToolbox H.264 1920×1080 at 60 fps, Opus/AVFoundation,
and media position 2.583 seconds. At 8.728 seconds the window became occluded;
playback correctly paused at 6.333 seconds. The display clock and sleep assertion
stopped, and draw/event/wakeup counters remained unchanged from the ten-second
checkpoint through exit. Catalog notifications and resets remained zero.

The two anonymous source summaries contain three completed 1 MiB ranges, no
errors/cancellations and no separate size fetches. One source's two ranges took
237,118 microseconds in total (maximum/first completion 188,905 microseconds);
the other source's one range took 166,762 microseconds. These are request-attempt
wall times through validated body completion, not an allocation of total startup
latency. The diagnostic intentionally records no URL, host, track identity,
request offsets or response content.

[Sanitized release log](evidence/2026-09-29-scoped-media-release-timing.log).
This is a functional transport/occlusion observation, **not** steady 60-second
playback or a performance pass. It does not explain the previous debug delay.
Eight VO drops and zero decoder drops were observed before the occlusion;
no whole-process CPU/RSS or A/V synchronization measurement was collected.

## Bounded macOS DNS helper

Current scoped-transport code moves all native DNS-SD calls into the first-party
`serein-dns` executable. `NetworkConfig::new` validates an explicitly supplied
absolute helper path without filesystem I/O; the app validates that executable
before opening its UI. It never discovers a helper through PATH. The default
direct-media path does not need this experimental transport's helper.

One lookup launches one short-lived process for both address families. A global
admission cap permits four active supervisors/children; excess requests fail busy
without queuing. Each supervisor owns its child until killed/reaped, independent
of the HTTP reader's current-thread Tokio runtime. Dropping the resolver future
closes a cancellation socket. The owner checks cancellation before spawning and
before sending a request, then waits on pipe/cancellation readiness against a
three-second budget. Error, timeout, excess output, cancellation and teardown
all kill and reap the exact owned process. The permit is released only afterward.
There is no persistent DNS service or idle polling thread.

Before reading input or calling native DNS, the helper installs a kqueue
parent-exit watch with a parent-ID race recheck and a2.8-second watchdog. That
watchdog exits the entire helper even if DNS-SD initialization is synchronously
blocked. Parent process cancellation remains an independent kill path. These
mechanisms bound userspace stalls; kernel process creation/kill/reaping cannot
be given an absolute real-time guarantee. At most four resources remain admitted
if the OS itself cannot reap a process.

The clean-environment, argument-free helper receives at most253 hostname bytes
plus newline through stdin. It returns only a versioned binary list of at most64
public IP addresses (maximum1093 valid bytes,2048-byte transport ceiling).
Both helper and parent reject nonpublic answers; the parent also rejects malformed,
duplicate and trailing data. No URL, signed query, cookie, header, account data,
hostname or address is logged. Failure is a nonzero exit with no output. The
existing native resolver uses the same system DNS-SD API and interface zero;
the helper's distinct process identity means scoped and per-app VPN routing
must still be tested. No alternate resolver fallback is introduced.

Validation: network unit tests pass22 with one explicit legacy DNS smoke ignored,
and three actual-binary integration tests pass. Tests cover a hung fake helper,
oversized/malformed output, kill/reap, cancellation after runtime destruction,
already-cancelled admission, cap4, protocol filtering, actual watchdog startup
timeout, and parent death while another process still holds the input pipe open.
Strict Clippy passes. A bounded native helper lookup of `example.com` returned
four public addresses in49 protocol bytes and no stderr. Its debug Mach-O links
libSystem and libiconv only; packaged closure/signing and live-media results are recorded below and in
packaging.md. Whole-process measurements must include this executable.


## DNS-backed release playback

The `bc2e53f` release app and matching first-party DNS helper passed a 40-second
public guest playback check through `--scoped-media`. The paired original binary
hashes, command, counters and scope are in the [result JSON](evidence/2026-09-29-dns-backed-scoped-playback.json);
the [native log](evidence/2026-09-29-dns-backed-scoped-playback.log) contains no
resolved stream URLs or credentials. At five seconds the app reported actual
VideoToolbox H.264 1920×1080 at 60 fps, Opus/AVFoundation and position 2.567 seconds.
It remained Playing through exit, reached 37.517 seconds, and recorded no occlusion
or engine error. Catalog changes/resets stayed zero; two persistent GPU targets
occupied 11,663,872 bytes. Nineteen VO drops and zero decoder drops were observed.

The two source summaries report 20 and one validated 1 MiB ranges respectively.
The first source records 21 completed attempts, including one cancellation/error;
completed attempts do not all mean validated payload. Their total attempt times
were 1,142,115 and 219,636 microseconds, with maxima 380,411 and 219,636. The summary
does not identify the cancellation's cause. This verifies the new native DNS
process path in actual media use, with successful app exit. It does not qualify
scoped/per-app VPN behavior, DNS packet egress, whole-process resource use,
sustained frame pacing or A/V synchronization. Other builds and diagnostic
preparation overlapped; this was a functional run.
