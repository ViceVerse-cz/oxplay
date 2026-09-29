# Playback timestamps in public links

Pasted links and `--url` accept an initial position from one query parameter
`t=90` or `start=90`, or the fragment `#t=90`. Decimal seconds and ordered
hour/minute/second components such as `1h2m3s`, `2m` and `90s` are supported.
Zero is valid. Components may be omitted, but cannot repeat or change order.
Only whole nonnegative seconds up to `u32::MAX` are accepted. Empty, signed,
fractional, exponent and overflowing values are rejected.

Multiple timestamp parameters, or both a query timestamp and `#t=`, are errors
even if the values agree. The supported timestamp fragment must contain only
`t=value`; unrelated fragments do not set a position. An invalid timestamp on
a watch link containing a playlist never silently changes the action into
opening the playlist.

The domain parser validates the public video identity independently of the
timestamp. The supervised guest resolver receives only that video identity;
the original query and fragment are never passed through as helper options or
interpreted as signed media/authentication. The requested start travels with
the exact worker request and result. Manual request retry retains it, stale
generations cannot publish it, and a pending account-stop handoff retains it.
Initial native installation passes the position directly to the existing
loadfile path. There is no delayed seek or new polling timer.

Normal card selections and explicit **Retry from start** use zero. Quality and
expiry replacement continue to use their freshly acknowledged native position,
not the original link's timestamp. CLI startup uses the same search adapter
after local preference hydration.

Focused source regressions were added for syntax/conflict handling, rejection
instead of playlist fallback, and position retention through manual retry.
They were not executed in this implementation pass. No live/native timestamp
playback or performance qualification is claimed.
