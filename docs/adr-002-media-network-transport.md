# ADR 002 — origin-scoped compressed media transport

Status: implementation in progress; not qualified as the default transport.

`serein-network` now implements bounded guest ranges, cancellation, strict response
checks, public-address filtering and per-hop origin policy. `serein-media::streams`
provides the libmpv callback boundary; `--scoped-media` selects the experimental
app integration. Seven deterministic HTTP/policy tests and strict Clippy passed.
The first live probe found that Hickory 0.26.3 cannot parse this macOS host's
scoped IPv6 nameserver address; a native cancellable system-DNS adapter is being
implemented rather than silently changing resolvers. Native/public playback and
resource qualification of this path remain pending. See [implementation notes](media-network.md).

The extractor returns separate, signed video/audio URLs and bounded headers tied
to each exact origin. libmpv's global HTTP-header option cannot demonstrate that
those headers remain confined across two tracks, redirects or segment requests.
The existing direct-CDN path has not passed the whole-process network-policy
gate. Cookies must never be placed in global media options. Until origin scoping
is enforced, a provider requiring headers that cannot be honored must report an
unsupported capability rather than silently dropping them.

The candidate is one in-process Rust HTTP range transport, exposed to libmpv
through `mpv_stream_cb_add_ro` under an opaque application protocol. This carries
compressed media bytes, not decoded frames; video decoding and the existing GPU
presentation path remain in libmpv. It introduces no HTTP server, browser,
separate UI, or second media engine. The existing direct-CDN path is the
performance comparison, not evidence of complete credential/proxy confinement.

The actual installed API 2.5 header, corresponding to
[mpv v0.41.0 stream_cb.h](https://github.com/mpv-player/mpv/blob/v0.41.0/libmpv/stream_cb.h),
provides open, blocking read, absolute seek, size, close, and nonblocking cancel
callbacks. Short reads are allowed. A zero-length read means final EOF and must
never mean temporary lack of data. The cancel callback may execute on a separate
thread and must not wait for a reader lock. No callback may call the same libmpv
instance. Protocol registrations and stream cookies outlive client handles and
remain owned through `mpv_terminate_destroy`. The header explicitly marks this
API unstable: the selected native version and actual source layout must remain
pinned/audited, rather than assuming future ABI compatibility.

The implementation boundary should be a media-owned, thread-safe stream factory
and per-stream reader/cancellation owner, with the HTTP implementation in the
network-policy layer. Registrations contain opaque IDs only; signed URLs,
headers and account material stay in bounded private Rust state. Each HTTP hop
must validate its destination and re-evaluate allowed headers. A redirect cannot
inherit another origin's authorization. Video/audio get distinct registrations.
Initial support should explicitly accept progressive HTTP-range tracks; HLS/DASH
manifests and arbitrary nested URLs remain unsupported until every subsequent
request can traverse the same policy.

Required invariants before enabling this path:

- No network or compressed-data copying on the UI thread. Callback reads may
  block their demux thread, with cancellation that interrupts current/future
  reads and seeks, including opening a stream. All open/read deadlines are
  bounded; teardown never waits indefinitely for a socket.
- Fixed per-stream compressed buffering and a fixed maximum active-stream count;
  return short reads rather than allocating the size libmpv happens to request.
  libmpv's own demux cache remains separately bounded and accounted for.
- Validate status, Content-Range start/end/total, length and representation
  consistency. Seeking to zero at open must obey the API's seekability probe.
  Reject non-seekable servers explicitly when needed instead of downloading the
  entire object. A read error never silently becomes a successful EOF.
- Account disconnection, expiry, replacement and stop invalidate registrations
  and pending results. Cancellation is independent of a blocked reader mutex.
  Close releases the cookie once, after callbacks can no longer use it.
- Local deterministic redirect/range/cancellation tests include cross-origin
  header leakage, wrong ranges, oversized responses, retry limits and network
  loss. Authorized live tests remain local; no cookies enter logs or fixtures.
- Compare the same public stream and local deterministic HTTP fixture against
  direct libmpv requests for CPU, memory, stalls and seeking. Measure the entire
  process tree, proxy/DNS/redirect/failure behavior and signed-URL refresh before
  claiming network confinement or passing the optimized gate.

This proposal is a response to a concrete origin-policy gap. It is not a claim
that wrapping one HTTP client automatically confines ffmpeg, extractors, account
helpers or arbitrary protocols. Those require explicit policy and independent
whole-process egress evidence.

## Implemented media boundary

`crates/media/src/streams.rs` exposes StreamFactory, StreamReader, StreamCancel,
OpenedStream and a sanitized four-variant StreamError. `Player::load_streams_at`
registers video/audio under opaque IDs; media sees neither original URLs nor
headers. Four registration slots and four simultaneously open readers are the
hard limits. Each read callback exposes at most64KiB of the mpv-owned compressed
buffer and validates the returned length. This is compressed input, not a video
frame or CPU presentation path.

Factories and initial seek(0) perform no I/O. The exact0.41 stream_cb.c invokes
seek(0) before registering cancellation, making this stronger requirement
necessary. Later size/read calls may perform bounded cancellable work on the
demux thread. Cancel owns a separate thread-safe handle and never waits for the
reader mutex. Callback panics are caught before the C boundary, and errors are
never converted to EOF. Callbacks never invoke the same mpv instance.

Stop/replacement invalidates IDs and cancels current readers. New registrations
are prepared, submitted and committed transactionally; submission failure rolls
back only the new IDs. A stable boxed registry lives in Player::Inner through
mpv_terminate_destroy; close uniquely reclaims each cookie. Native headless tests
verify actual demux reads and zero remaining cookies after teardown, bounded
reads/readers, stale-ID rejection, and cancellation of a blocked reader.

The app adapter selects this path explicitly with `--scoped-media`; network
policy/DNS/redirect, real HTTPS playback, and performance qualification remain
separate. Existing access-references=no maps to FFmpeg's rejecting nested
io_open hook for the reviewed lavf path; malformed/reference-container egress
and all alternate demuxers still need negative testing. Direct HTTPS fallback
now always enables tls-verify=yes and accepts a prevalidated explicit CA resource.
mpv0.41's unsafe defaultfalse is never inherited. Native option verification is
a regression test; synthetic loopback trust-chain rejection, explicit-CA
acceptance and hostname-mismatch rejection also pass (media-tls.md).

## macOS DNS initialization isolation

The HTTP transport remains in-process, but macOS native DNS lookup now uses a
short-lived first-party `serein-dns` helper. Source review showed synchronous
DNSServiceGetAddrInfo initialization could outlive Tokio's async timeout; merely
moving it to an uncancellable blocking thread would not solve shutdown ownership.
The helper has an independent parent-death/deadline watchdog, while a bounded
parent supervisor owns kill/reap and survives HTTP runtime cancellation. Four
lookups maximum are admitted with no unbounded queue. A small versioned binary
protocol carries only a validated hostname request and public-address response.
There is no persistent service, media engine or second UI in this helper.

The executable path is explicit and validated before UI startup; packaging must
ship, sign and inventory it, and resource sampling must include its transient
processes. This narrow process exception isolates native initialization while
using the same system DNS-SD API and interface zero. The helper's distinct
process identity still requires per-app VPN and scoped-routing validation.
It does not qualify VPN,
proxy, whole-process egress, performance or other operating systems by itself.
See [media-network.md](media-network.md) for implementation bounds and tests.
