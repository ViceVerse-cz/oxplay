# Security reporting

This is an experimental development build, with no supported stable releases
or security-response SLA yet. Account and privacy claims remain gated by the
tests in [SPEC.md](SPEC.md) and [docs/progress.md](docs/progress.md).

Do not post secrets or an exploitable private disclosure in a public issue.
If the repository host offers private vulnerability reporting, use that
repository-specific channel after verifying it is enabled. No private mailbox
or reporting endpoint has been designated in this source tree yet. If no
private channel is available, open only a minimal request for private contact,
with no vulnerability details or sensitive attachments. Maintainers must
publish a working private reporting route before a public stable release.

Useful sanitized reports describe the affected application/dependency revision,
OS, impact, preconditions, and a minimal reproduction using synthetic data.
Never attach real cookies, tokens, signed playback URLs, passwords, private
provider responses, complete environment dumps, raw memory dumps or packet
captures. Do not test with another person's account or perform account writes
without explicit authorization.

Priority areas include credential storage/import/redaction, stale authenticated
results after sign-out, URL/redirect policy, helper argument/config isolation,
process-tree cleanup, malformed metadata, media FFI ownership and update trust.
Do not assume a crash in a native dependency is harmless. Reproduce safely and
coordinate upstream disclosure without revealing user data.

Review fixes with regression evidence and document affected versions. Preserve
the reporter's privacy and credit preference. Avoid publishing exploit details
before a coordinated remediation decision.
