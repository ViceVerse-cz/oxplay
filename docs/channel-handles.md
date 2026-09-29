# Public channel handles

The shared search field accepts a standalone `@handle` or a strict HTTPS
`youtube.com/@handle` URL. `www.youtube.com` and `m.youtube.com` are also
accepted. Videos, Shorts, Streams and Playlists URL suffixes select their
corresponding channel tab. A phrase such as `@name tutorial` remains a search.

Handle lookup uses the existing supervised guest yt-dlp catalog worker, with
its cancellation, output/time bounds, page size, retry policy and ad filtering.
Connecting an account does not attach credentials to this lookup. No extra
HTTP client, persistent helper or synthetic channel identity is introduced.

A successful response must supply a valid stable `UC…` channel ID and real
channel metadata. The acknowledged route and continuation cursor switch to
that ID immediately. Further pages, tabs and local following use this stable
identity; a subsequent handle reassignment does not redirect pagination.
Lookup failure leaves the normal cancellable catalog error/retry flow.

The typed handle is an address, not proof of ownership or account identity.
Its parser supports Unicode and percent-encoded UTF-8 with bounded input. It
rejects credential-bearing/non-YouTube URLs, nondefault ports, queries,
fragments, extra path components, encoded separators, controls and path
rewrites. An address is regenerated under the fixed HTTPS YouTube origin.
The 100-scalar/400-byte handle envelope is an application safety bound, not
YouTube registration eligibility. YouTube remains authoritative for allowed
scripts and names.

[YouTube's handle documentation](https://support.google.com/youtube/answer/11585688?hl=en)
describes international handles, variable script-dependent lengths and encoded
URLs. It was checked during implementation on 2026-09-29.

Small source regressions cover Unicode address round trips, malformed/unsafe
addresses, search-versus-handle routing and stable-ID continuation. They were
added but not executed in this fast implementation pass. Live provider,
keyboard, accessibility and packaged behavior remain unqualified for this
feature; no performance testing was performed.
