#!/usr/bin/env python3
"""Synthetic libmpv TLS rejection/acceptance test; binds loopback only, no real credentials."""
import contextlib
import http.server
import json
import os
from pathlib import Path
import shutil
import signal
import ssl
import struct
import subprocess
import tempfile
import threading
import time
import sys

ROOT = Path(__file__).resolve().parents[1]


class Commands:
    """Keep a private fixture until every owned process group is confirmed gone."""
    def __init__(self):
        self.safe_to_remove = True

    def run(self, args, *, env=None, timeout=20, quiet=False):
        if not self.safe_to_remove:
            raise RuntimeError("Previous TLS command cleanup is unconfirmed")
        if not all(hasattr(os, name) for name in ("waitid", "WNOWAIT", "WEXITED", "WNOHANG")):
            raise RuntimeError("TLS harness requires unreaped-leader process supervision")
        process = subprocess.Popen(args, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.DEVNULL if quiet else None,
                                   stderr=subprocess.DEVNULL if quiet else None,
                                   start_new_session=True)
        self.safe_to_remove = False
        deadline = time.monotonic() + timeout
        try:
            while True:
                # Do not poll()/wait() before group signaling: retaining the
                # unreaped leader pins its PID/PGID against reuse.
                status = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT | os.WNOHANG)
                if status is not None:
                    if status.si_code != os.CLD_EXITED or status.si_status != 0:
                        raise RuntimeError("TLS prerequisite or integration test failed")
                    return
                if time.monotonic() >= deadline:
                    raise TimeoutError("TLS prerequisite or integration test timed out")
                time.sleep(0.02)
        finally:
            reap_group(process)
            self.safe_to_remove = True


def reap_group(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        raise RuntimeError("TLS child could not be reaped; private fixture retained") from None
    deadline = time.monotonic() + 2
    while True:
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            return
        except PermissionError:
            pass  # A permission error is not positive evidence of disappearance.
        if time.monotonic() >= deadline:
            raise RuntimeError("TLS process group remains; private fixture retained")
        time.sleep(0.01)


@contextlib.contextmanager
def fixture_directory(commands):
    directory = Path(tempfile.mkdtemp(prefix="oxplay-tls-"))
    try:
        yield directory
    finally:
        if commands.safe_to_remove:
            shutil.rmtree(directory)
        else:
            print(f"Unconfirmed child cleanup; private synthetic TLS fixture retained: {directory}",
                  file=sys.stderr)


def openssl(commands, *args):
    commands.run([shutil.which("openssl") or "openssl", *args], quiet=True)


def certificate(commands, directory, name, san):
    key, csr, cert = (directory / f"{name}.{suffix}" for suffix in ("key", "csr", "pem"))
    ext = directory / f"{name}.ext"
    ext.write_text(f"subjectAltName={san}\nextendedKeyUsage=serverAuth\n")
    openssl(commands, "req", "-new", "-newkey", "rsa:2048", "-nodes", "-sha256", "-subj", f"/CN={name}",
            "-keyout", str(key), "-out", str(csr))
    openssl(commands, "x509", "-req", "-in", str(csr), "-CA", str(directory / "ca.pem"),
            "-CAkey", str(directory / "ca.key"), "-CAcreateserial", "-days", "1",
            "-sha256", "-extfile", str(ext), "-out", str(cert))
    return key, cert


def wav():
    data = bytes(48_000)
    return (b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt "
            + struct.pack("<IHHIIHH", 16, 1, 1, 48_000, 96_000, 2, 16)
            + b"data" + struct.pack("<I", len(data)) + data)


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = False
    allow_reuse_address = False

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(3)
        try:
            return self.tls.wrap_socket(connection, server_side=True), address
        except Exception:
            connection.close()
            raise

    def handle_error(self, request, address):
        # Expected peer-aborted handshakes contain no useful test evidence.
        pass


@contextlib.contextmanager
def server(key, cert):
    requests = []
    payload = wav()

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            if self.path != "/fixture.wav":
                self.send_error(404)
                return
            start, end = 0, len(payload) - 1
            requested = self.headers.get("Range")
            if requested:
                try:
                    left, right = requested.removeprefix("bytes=").split("-", 1)
                    if not requested.startswith("bytes=") or not left.isdecimal():
                        raise ValueError()
                    start = int(left)
                    end = min(int(right), end) if right else end
                    if not 0 <= start <= end < len(payload):
                        raise ValueError()
                except ValueError:
                    self.send_error(416)
                    return
            requests.append((start, end))
            self.send_response(206 if requested else 200)
            self.send_header("Content-Type", "audio/wav")
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(end - start + 1))
            if requested:
                self.send_header("Content-Range", f"bytes {start}-{end}/{len(payload)}")
            self.end_headers()
            self.wfile.write(payload[start:end + 1])

    httpd = Server(("127.0.0.1", 0), Handler)
    try:
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.minimum_version = ssl.TLSVersion.TLSv1_2
        tls.load_cert_chain(cert, key)
        httpd.tls = tls
    except BaseException:
        httpd.server_close()
        raise
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"https://127.0.0.1:{httpd.server_port}/fixture.wav", requests
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join(timeout=2)
        if thread.is_alive():
            raise RuntimeError("fixture server did not stop")


def interrupted(_signal, _frame):
    raise KeyboardInterrupt


def main():
    previous_umask = os.umask(0o077)
    previous_term = signal.signal(signal.SIGTERM, interrupted)
    commands = Commands()
    try:
        with fixture_directory(commands) as directory:
            openssl(commands, "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-sha256", "-days", "1",
                    "-subj", "/CN=Oxplay disposable fixture CA", "-keyout", str(directory / "ca.key"),
                    "-out", str(directory / "ca.pem"))
            good = certificate(commands, directory, "good", "IP:127.0.0.1")
            bad_host = certificate(commands, directory, "mismatched-host", "DNS:invalid.fixture.test")
            with (server(*good) as (good_url, good_requests),
                  server(*good) as (untrusted_url, untrusted_requests),
                  server(*bad_host) as (bad_url, bad_requests)):
                env = os.environ.copy()
                for key in list(env):
                    if key.lower().endswith("_proxy") or key.startswith("OXPLAY_"):
                        del env[key]
                env.update(OXPLAY_TLS_FIXTURE_GOOD=good_url, OXPLAY_TLS_FIXTURE_UNTRUSTED=untrusted_url,
                           OXPLAY_TLS_FIXTURE_MISMATCH=bad_url,
                           OXPLAY_TLS_FIXTURE_CA=str(directory / "ca.pem"))
                commands.run(["cargo", "test", "--locked", "--offline", "-p", "oxplay-media",
                    "tls_tests::synthetic_loopback_certificate_and_hostname_verification", "--",
                    "--ignored", "--exact", "--test-threads=1"], env=env, timeout=180)
                if not good_requests or untrusted_requests or bad_requests:
                    raise RuntimeError("unexpected fixture HTTP access after certificate verification")
                print(json.dumps({"untrusted_chain_rejected": True, "explicit_ca_accepted": True,
                    "hostname_mismatch_rejected": True, "trusted_http_requests": len(good_requests),
                    "untrusted_http_requests": len(untrusted_requests),
                    "mismatched_host_http_requests": len(bad_requests), "network_scope": "loopback only"}))
    finally:
        signal.signal(signal.SIGTERM, previous_term)
        os.umask(previous_umask)


if __name__ == "__main__":
    main()
