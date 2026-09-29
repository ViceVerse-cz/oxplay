# Explicit external integration checks

All four explicit checks below **passed on 2026-09-29**, against source
`d7c58acd0096c7b767377f29f423e1ed79668c2a`, after the coordinated measurement
hold ended. They remain ignored in ordinary workspace tests because they have
different host/network effects. This includes the added production-DNS helper
check and the strengthened synthetic TLS/Keychain assertions.

| Check | Supervised elapsed time | Observed result |
| --- | ---: | --- |
| [Direct libmpv TLS](evidence/2026-09-29-external-tls.log) | 2.296s | Trusted HTTP requests: 1; untrusted-chain and wrong-host requests: 0 each |
| [Synthetic Keychain](evidence/2026-09-29-external-keychain.log) | 1.007s | Reopen/rewrite succeeded; envelope absent, reopened session empty and exact synthetic Keychain item absent after deletion |
| [System DNS primitive](evidence/2026-09-29-external-dns-primitive.log) | 5.524s | Native public resolution and owned-descriptor cleanup passed; elapsed includes compilation |
| [Production DNS helper](evidence/2026-09-29-external-dns-helper.log) | 0.475s | Actual helper returned successful bounded protocol; addresses were not logged |

[Source/toolchain/executable hashes and complete result provenance](evidence/2026-09-29-external-integration.json)
identify these runs. Every rerun finished with exit 0 and confirmed absence of
its supervised process group. No real credentials/account were used. The
Keychain result includes in-test verification of both envelope and key deletion;
it is not inferred merely from a destructor running. These are functional
debug-test checks, not packaged-release, hardware-decoder, performance, proxy,
account or other-platform qualification.

The [first TLS attempt](evidence/2026-09-29-external-tls-attempt1.log) passed its
Rust/TLS assertions, but the separate temporary evidence wrapper then raised
`PermissionError` while signaling the exited, unreaped leader's group. That
outer cleanup was unconfirmed, so it is retained as an incomplete harness run.
The temporary wrapper was corrected to treat permission failure as uncertainty,
reap, and require positive group absence; the complete TLS check was rerun.
No checked-in production or test source was changed during these executions.

Run from the repository root after the measurement hold, with the locked
toolchain/dependencies and native libmpv build available. Do not use a blanket
`--ignored` command: these checks have different host/network effects.

## Direct libmpv TLS

```sh
python3 scripts/test_media_tls.py
```

Requires Python with TLS, OpenSSL, Cargo's already-fetched offline dependency
cache, and the installed/linkable production libmpv. The harness generates a
disposable CA and two leaf certificates in a private temporary directory. Three
ephemeral loopback HTTPS listeners distinguish trusted, untrusted-chain, and
trusted-chain/wrong-host attempts. No certificate is installed in a trust store.
The test uses the production `Player::new_with_ca_file` and native URL-loading
path, with explicit null audio/video outputs and a silent generated WAV. It does
not need a window, hardware decoder, browser, extractor, account, or external
HTTP service. Ambient proxy and `SEREIN_*` variables are removed from the Cargo
child environment; the fixture then supplies only its explicit loopback inputs.

The Rust check has a 12-second outcome deadline per attempt. Its wrapper uses
20-second OpenSSL command budgets and a 180-second Cargo budget. Previously a
Cargo timeout killed only the leader and could leave the Rust test running.
Commands now run in owned sessions; `waitid(WNOWAIT)` preserves the leader until
the process group is signaled, then the leader is reaped and group absence is
confirmed. Permission failure is not absence. Unconfirmed cleanup retains the
private fixture instead of deleting files while a child may still use them.
SIGTERM and ordinary interruption enter cleanup; SIGKILL/power loss cannot offer
that guarantee. Kernel process operations have no absolute real-time bound.
The loopback servers stop accepting and join their request threads; sockets have
three-second timeouts. Keys/certificates are removed after confirmed normal
cleanup, never claimed forensically erased.

Success now requires a real trusted HTTP request and **zero HTTP requests on
each rejected server**, rather than combining trusted/untrusted counters. This
qualifies direct libmpv certificate/hostname checking for this native build only.
It does not qualify Rustls scoped-media transport, production CA packaging,
redirect/header confinement, proxy/DNS protection, or active hardware decoding.

Offline harness regressions (no servers, OpenSSL, Cargo, or network):

```sh
python3 -m unittest discover -s scripts -p test_media_tls_harness.py
```

## Synthetic macOS Keychain

```sh
cargo test --locked --offline -p serein-storage vault::tests::macos_keychain_synthetic_roundtrip -- --ignored --exact --test-threads=1
```

Requires macOS and access to the user's local login Keychain. The OS may present
an access prompt. The check uses a new random application profile, first refuses
any existing matching item, and writes only explicitly synthetic bytes. It never
scans profiles, imports cookies, reads another account item, or contacts YouTube.
The actual production vault encrypts its session envelope in a private temporary
directory and stores its random key in Keychain.

The strengthened check reopens the vault through its public constructor,
verifies the session, rewrites it using the same protected key with a fresh nonce,
and verifies the replacement. Successful deletion must remove the envelope and
Keychain item; the reopened store must observe no session. An unwind guard also
attempts cleanup and reports failure without exposing profile/key bytes. OS
denial, process termination, or machine failure can leave a synthetic item;
cleanup is not guaranteed merely because a guard exists. This checks persistence
and deletion, not login, identity verification, expiry, or account acceptance.

## macOS system DNS: primitive versus production helper

The original ignored smoke is deliberately renamed to describe its actual scope:

```sh
cargo test --locked --offline -p serein-network dns_macos::tests::native_resolution_and_query_drop_close_the_owned_socket -- --ignored --exact --test-threads=1
```

It contacts the configured system resolver for `example.com`, checks returned
addresses with the production public-address policy, and verifies that dropping
an owned DNS-SD query closes its descriptor. It runs the primitive **inside the
test process**, whereas current macOS media runs it in the supervised first-party
`serein-dns` executable. Its async three-second deadline does not bound a
synchronously stuck native DNS initialization call. Run it under an external
process supervisor when collecting evidence; do not call this a production
helper-cancellation test.

The new complementary check uses Cargo's exact built executable path:

```sh
cargo test --locked --offline -p serein-network --test dns_helper live_system_dns_helper_returns_bounded_public_protocol -- --ignored --exact --test-threads=1
```

It launches the actual helper with a cleared environment, no arguments, and only
`example.com` on stdin. It requires success within five seconds, empty stderr,
bounded `SDN1` framing, recognized address families, unique answers, and no
trailing bytes. Outputs are read with a byte cap and nonblocking descriptors
after exit. Answer values are never logged. The real helper applies the full
public-address filter and native watchdog; the test additionally rejects basic
loopback/private/link-local IPv4, unspecified, and multicast answers. Network
policy can legitimately reject the host resolver's response, making this check
fail rather than substitute another resolver.

The helper is killed/reaped on early test failure. This check establishes the
actual binary's successful native DNS/protocol path, not its application parent
supervisor. Existing deterministic `dns_process` tests separately cover the
parent's admission, cancellation after runtime destruction, protocol rejection,
timeout and child reaping; ordinary `dns_helper` integration tests cover malformed
offline input, the startup watchdog, and parent death. Neither live DNS check
makes HTTP requests or accesses account data. VPN/per-app routing and packaged
helper provenance require their separate recorded validation.
