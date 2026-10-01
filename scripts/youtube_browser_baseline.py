#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite, isolated real-YouTube-site comparison; never uses a personal profile.

Prepare the pinned browser with browser_baseline.py first. This script is an
explicit online diagnostic. It measures the whole owned browser process tree,
requires matched physical geometry and records the actual selected decoder;
the default matched-codec mode also requires hardware decoding. It
exports only bounded numeric state, fixed codec names and a public video ID.
"""
import argparse
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import re
import select
import signal
import socket
import subprocess
import sys
import tempfile
import time
from urllib.parse import parse_qs, urlsplit

import browser_baseline as browser
import measure

VIDEO_ID = re.compile(r"[A-Za-z0-9_-]{11}\Z")
CODECS = {"h264", "vp9", "av1"}
SAFE_DECODERS = {"VideoToolboxVideoDecoder", "FFmpegVideoDecoder", "VpxVideoDecoder",
                 "Dav1dVideoDecoder", "GpuVideoDecoder", "MojoVideoDecoder"}


class AdmissionComplete(Exception):
    """Internal control flow; cleanup and its failure gate still run."""


def public_video(url):
    parsed = urlsplit(url)
    if (parsed.scheme != "https" or parsed.username or parsed.password or parsed.port
            or parsed.fragment or len(url) > 256):
        raise ValueError("Only an explicit public HTTPS YouTube video URL is admitted")
    if parsed.hostname in ("youtube.com", "www.youtube.com") and parsed.path == "/watch":
        query = parse_qs(parsed.query, keep_blank_values=True, strict_parsing=True)
        if set(query) != {"v"} or len(query["v"]) != 1:
            raise ValueError("Use a single-video watch URL without other parameters")
        identity = query["v"][0]
    elif parsed.hostname == "youtu.be" and not parsed.query:
        identity = parsed.path.removeprefix("/")
    else:
        raise ValueError("Only youtube.com/watch or youtu.be video URLs are admitted")
    if not VIDEO_ID.fullmatch(identity):
        raise ValueError("Invalid public video ID")
    return identity, "https://www.youtube.com/watch?v=" + identity + "&hl=en"


def source_digest(value):
    if not isinstance(value, str) or len(value) > 65536:
        raise ValueError("Invalid media source identity")
    return hashlib.sha256(value.encode()).hexdigest()


class OnlineCDP(browser.CDP):
    """Associate the current DOM video with live CDP players, without URL logs."""
    def __init__(self, reader, writer):
        super().__init__(reader, writer)
        self.live = set()
        self.source_hashes = {}
        self.epochs = {}

    def event(self, message):
        if message.get("sessionId") != self.media_session:
            return
        method, params = message.get("method"), message.get("params", {})
        if method == "Media.playerCreated":
            identity = params["player"]["playerId"]
            self.live.add(identity)
        if method == "Media.playerEventsAdded":
            identity = params["playerId"]
            for event in params["events"]:
                value = json.loads(event["value"])
                if value.get("event") == "kLoad":
                    self.source_hashes[identity] = source_digest(value.get("url"))
                    self.live.add(identity)
                    self.epochs[identity] = self.epochs.get(identity, 0) + 1
                elif value.get("event") == "kWebMediaPlayerDestroyed":
                    self.live.discard(identity)
                    self.source_hashes.pop(identity, None)
                    self.epochs[identity] = self.epochs.get(identity, 0) + 1
        if method == "Media.playerPropertiesChanged":
            identity = params["playerId"]
            previous = self.players.get(identity, {})
            for prop in params["properties"]:
                if (prop["name"] in ("kVideoDecoderName", "kIsPlatformVideoDecoder",
                                    "kVideoTracks", "kAudioTracks", "kResolution")
                        and previous.get(prop["name"]) != prop["value"]):
                    self.epochs[identity] = self.epochs.get(identity, 0) + 1
        # The shared bounded parser discards all non-whitelisted properties and
        # raw event URLs. Source hashes remain private in memory, not evidence.
        super().event(message)

    def current_player(self, node, digest):
        direct = [identity for identity in self.live
                  if self.player_nodes.get(identity) == node and identity in self.players]
        if len(direct) == 1:
            return direct[0], "backend_dom_node"
        exact = [identity for identity in self.live if identity in self.players
                 and self.source_hashes.get(identity) == digest]
        if len(exact) == 1 and self.player_nodes.get(exact[0]) is None:
            return exact[0], "sole_live_player_exact_current_source"
        raise ValueError("Current site video has no unique live hardware-player association")

    def association_diagnostics(self, node, digest):
        """Bounded counts only; player IDs, sources and DOM IDs stay private."""
        candidates = [identity for identity in self.live if identity in self.players]
        return {"live_player_count": min(len(self.live), 8),
                "live_property_player_count": min(len(candidates), 8),
                "dom_association_count": min(sum(self.player_nodes.get(i) == node for i in candidates), 8),
                "exact_source_count": min(sum(self.source_hashes.get(i) == digest for i in candidates), 8),
                "source_available": isinstance(digest, str) and len(digest) == 64,
                "players": [decoder_diagnostics(self.players[i]) for i in sorted(candidates)[:8]]}

    def drain_idle_events(self):
        """Bound idle pipe work; retain every partial/unprocessed frame."""
        start = time.monotonic()
        messages = received = parsed = 0
        budget_hit = False
        while True:
            remaining = 256 * 1024 - received - parsed
            if messages >= 256 or remaining <= 0 or time.monotonic() - start >= 0.005:
                budget_hit = True
                break
            boundary = self.buffer.find(b"\0")
            if boundary >= 0:
                size = boundary + 1
                if size > remaining:
                    budget_hit = True
                    break
                message = json.loads(self.buffer[:boundary])
                # There is no outstanding command on this single-thread owner.
                # Preserve an unexpected reply rather than consuming/dropping it.
                if not isinstance(message, dict) or "id" in message:
                    raise ValueError("Unexpected idle CDP command response")
                self.event(message)
                del self.buffer[:size]
                parsed += size
                messages += 1
                continue
            if not select.select([self.reader], [], [], 0)[0]:
                break
            try:
                chunk = self.reader.recv(min(65536, remaining), socket.MSG_DONTWAIT)
            except BlockingIOError:
                break
            if not chunk:
                raise EOFError("Browser control pipe closed during idle drain")
            self.buffer.extend(chunk)
            received += len(chunk)
            if len(self.buffer) > 4 * 1024 * 1024:
                raise ValueError("CDP message bound exceeded")
        pending = bool(self.buffer) or bool(select.select([self.reader], [], [], 0)[0])
        return {"messages": messages, "received_bytes": received, "parsed_bytes": parsed,
                "buffered_bytes": len(self.buffer), "pending_input": pending,
                "partial_frame": bool(self.buffer) and b"\0" not in self.buffer,
                "budget_exhausted": budget_hit and pending,
                "elapsed_ms": (time.monotonic() - start) * 1000}


def decoder_diagnostics(player):
    """Partial evidence survives failed admission without echoing CDP data."""
    result = {"decoder_known": player.get("kVideoDecoderName") in SAFE_DECODERS,
              "video_decoder": player.get("kVideoDecoderName") if player.get("kVideoDecoderName") in SAFE_DECODERS else None,
              "platform_video_decoder": player.get("kIsPlatformVideoDecoder") == "true"}
    for kind, allowed in (("video", CODECS), ("audio", {"aac", "opus", "vorbis"})):
        raw = player.get("k" + kind.title() + "Tracks")
        try:
            tracks = json.loads(raw) if isinstance(raw, str) and len(raw) <= 65536 else None
        except (ValueError, TypeError):
            tracks = None
        result[kind + "_track_count"] = min(len(tracks), 32) if isinstance(tracks, list) else None
        track = tracks[0] if isinstance(tracks, list) and len(tracks) == 1 and isinstance(tracks[0], dict) else {}
        result[kind + "_codec"] = track.get("codec") if track.get("codec") in allowed else None
        if kind == "video":
            color = track.get("color space")
            result["sdr_transfer_known"] = isinstance(color, dict) and color.get("transfer") in ("BT709", "SRGB", "IEC61966_2_1")
            result["hdr_metadata_present"] = bool(track.get("hdr metadata"))
        else:
            for field, key, high in (("samples per second", "audio_sample_rate", 192000), ("channels", "audio_channels", 8)):
                value = track.get(field)
                result[key] = value if type(value) is int and 1 <= value <= high else None
    return result


def decoder_evidence(player):
    """Extract fixed allowlisted fields; never export arbitrary CDP strings."""
    tracks = json.loads(player.get("kVideoTracks", "null"))
    if not isinstance(tracks, list) or len(tracks) != 1 or not isinstance(tracks[0], dict):
        raise ValueError("Actual selected video track is unavailable or ambiguous")
    video = tracks[0]
    codec = video.get("codec")
    if codec not in CODECS:
        raise ValueError("Video codec is not admitted for this comparison")
    color = video.get("color space", {})
    if not isinstance(color, dict):
        raise ValueError("Invalid video color metadata")
    # Hardware 1080p60 SDR is the reference workload. HDR requires another gate.
    if (color.get("transfer") not in ("BT709", "SRGB", "IEC61966_2_1")
            or video.get("hdr metadata")):
        raise ValueError("SDR video metadata is not established")
    decoder = player.get("kVideoDecoderName")
    if decoder not in SAFE_DECODERS:
        raise ValueError("Unknown video decoder")
    audio = json.loads(player.get("kAudioTracks", "null"))
    if not isinstance(audio, list) or len(audio) != 1 or not isinstance(audio[0], dict):
        raise ValueError("Actual selected audio track is unavailable or ambiguous")
    audio = audio[0]
    if audio.get("codec") not in ("aac", "opus", "vorbis"):
        raise ValueError("Audio codec is not admitted")
    sample_rate, channels = audio.get("samples per second"), audio.get("channels")
    if (type(sample_rate) is not int or not 8000 <= sample_rate <= 192000
            or type(channels) is not int or not 1 <= channels <= 8):
        raise ValueError("Audio geometry is unavailable")
    return {"video_codec": codec, "video_decoder": decoder,
            "platform_video_decoder": player.get("kIsPlatformVideoDecoder") == "true",
            "sdr": True, "audio_codec": audio["codec"],
            "audio_sample_rate": sample_rate, "audio_channels": channels}


SNAPSHOT_JS = """(async () => {
const all=[...document.querySelectorAll('video')],v=all.find(v=>v===window.oxplayVideo);
if(!v)return {identity_ok:false};
const r=v.getBoundingClientRect(),q=v.getVideoPlaybackQuality(),p=document.querySelector('#movie_player');
const source_hash=[...new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(v.currentSrc)))].map(b=>b.toString(16).padStart(2,'0')).join('');
return {identity_ok:all.length===1&&v===window.oxplayVideo,
 video_id:new URL(location.href).searchParams.get('v'),
 selected_video_id:typeof p?.getVideoData==='function'?p.getVideoData().video_id:null,
 ad_showing:!!p?.classList.contains('ad-showing'),source_hash,
 consent_showing:[...document.querySelectorAll('button,[role=button]')].some(b=>b.getClientRects().length>0&&getComputedStyle(b).visibility==='visible'&&!b.disabled&&['reject all','odmítnout vše','alle ablehnen'].includes(b.textContent.trim().toLowerCase())),
 current_time:v.currentTime,duration:Number.isFinite(v.duration)?v.duration:null,
 creation_time:q.creationTime,time_origin:performance.timeOrigin,changes:window.oxplayChanges,
 playback_rate:v.playbackRate,in_view:r.left>=0&&r.top>=0&&r.right<=innerWidth&&r.bottom<=innerHeight,
 width:v.videoWidth,height:v.videoHeight,paused:v.paused,muted:v.muted,volume:v.volume,
 ready_state:v.readyState,error:v.error?.code??null,visibility:document.visibilityState,
 focused:document.hasFocus(),css_width:r.width,css_height:r.height,dpr:devicePixelRatio,
 viewport_scale:visualViewport.scale,total_frames:q.totalVideoFrames,dropped_frames:q.droppedVideoFrames};})()"""

INSTRUMENT_JS = """(() => {
const videos=[...document.querySelectorAll('video')];if(videos.length!==1)return false;
window.oxplayVideo=videos[0];window.oxplayChanges=0;
const changed=()=>window.oxplayChanges++;
for(const e of ['pause','seeking','emptied','loadedmetadata','ratechange','volumechange','ended','waiting','stalled'])oxplayVideo.addEventListener(e,changed);
for(const e of ['blur','resize'])window.addEventListener(e,changed);
document.addEventListener('scroll',changed,true);
document.addEventListener('visibilitychange',changed);
let rect=oxplayVideo.getBoundingClientRect();
window.oxplaySizeObserver=new ResizeObserver(()=>{const next=oxplayVideo.getBoundingClientRect();if(next.width!==rect.width||next.height!==rect.height){rect=next;changed();}});
oxplaySizeObserver.observe(oxplayVideo);
const player=document.querySelector('#movie_player');let ads=player?.classList.contains('ad-showing');
window.oxplayObserver=new MutationObserver(()=>{const next=player.classList.contains('ad-showing');if(next!==ads){ads=next;changed();}});
if(player)oxplayObserver.observe(player,{attributes:true,attributeFilter:['class']});
return true;})()"""


def qualify(snapshot, decoder, identity, width, height, codec, remaining, comparison="matched-codec"):
    geometry_tolerance = 1.0 if comparison == "default-playback" else 0.1
    numbers = ("current_time", "duration", "creation_time", "time_origin", "playback_rate",
               "volume", "css_width", "css_height", "dpr", "viewport_scale",
               "total_frames", "dropped_frames", "changes")
    if any(type(snapshot.get(key)) not in (int, float) or not math.isfinite(snapshot[key])
           for key in numbers):
        raise ValueError("Missing or nonfinite playback state")
    if (snapshot.get("identity_ok") is not True or snapshot.get("video_id") != identity
            or snapshot.get("selected_video_id") != identity or snapshot.get("ad_showing") is not False
            or snapshot.get("consent_showing") is not False
            or snapshot.get("in_view") is not True or snapshot.get("playback_rate") != 1
            or snapshot.get("paused") is not False or snapshot.get("muted") is not False
            or snapshot["volume"] <= 0 or snapshot.get("error") is not None
            or snapshot.get("visibility") != "visible" or snapshot.get("focused") is not True
            or snapshot.get("ready_state", 0) < 3 or snapshot.get("width") != 1920
            or snapshot.get("height") != 1080 or snapshot["viewport_scale"] != 1
            or abs(snapshot["css_width"] * snapshot["dpr"] - width) > geometry_tolerance
            or abs(snapshot["css_height"] * snapshot["dpr"] - height) > geometry_tolerance
            or snapshot["duration"] - snapshot["current_time"] < remaining):
        raise ValueError("Visible unmuted 1080p content or matched physical geometry is not established")
    if comparison not in ("matched-codec", "default-playback"):
        raise ValueError("Unknown comparison scope")
    if comparison == "matched-codec" and (decoder["video_decoder"] != "VideoToolboxVideoDecoder"
            or not decoder["platform_video_decoder"] or decoder["video_codec"] != codec):
        raise ValueError("Matched codec and actual hardware decoder are not established")


def sanitized_snapshot(snapshot):
    # Source identity is necessary for live association, never persisted.
    result = {}
    for key in ("current_time", "duration", "creation_time", "time_origin", "changes",
                "playback_rate", "width", "height", "volume", "ready_state", "error",
                "css_width", "css_height", "dpr", "viewport_scale", "total_frames", "dropped_frames"):
        value = snapshot.get(key)
        result[key] = value if type(value) in (int, float) and math.isfinite(value) else None
    for key in ("identity_ok", "ad_showing", "consent_showing", "in_view", "paused", "muted", "focused"):
        result[key] = snapshot.get(key) if type(snapshot.get(key)) is bool else None
    result["visibility"] = snapshot.get("visibility") if snapshot.get("visibility") in ("visible", "hidden") else None
    for key in ("video_id", "selected_video_id"):
        value = snapshot.get(key)
        result[key] = value if isinstance(value, str) and VIDEO_ID.fullmatch(value) else None
    return result


def qualify_online_interval(start, end, elapsed):
    """Use the shared continuity gate, then enforce online zero-drop admission."""
    frames, drops = browser.qualify_interval(start, end, elapsed)
    if drops != 0:
        raise ValueError("Online playback interval dropped video frames")
    return frames, drops


def launch(binary, profile, window_width=1320, window_height=860):
    send, child_read = socket.socketpair()
    child_write, receive = socket.socketpair()
    read_fd = fcntl.fcntl(child_read.fileno(), fcntl.F_DUPFD_CLOEXEC, 10)
    write_fd = fcntl.fcntl(child_write.fileno(), fcntl.F_DUPFD_CLOEXEC, 10)
    child_read.close()
    child_write.close()
    flags = [f"--user-data-dir={profile}", "--remote-debugging-pipe", "--lang=en-US",
             "--no-first-run", "--no-default-browser-check", "--disable-background-networking",
             "--disable-component-update", "--disable-sync", "--disable-default-apps",
             "--disable-domain-reliability", "--metrics-recording-only", "--no-pings",
             "--use-mock-keychain", f"--window-size={window_width},{window_height}", "about:blank"]
    try:
        process = subprocess.Popen([sys.executable, str(Path(browser.__file__).resolve()),
            "_launch", str(read_fd), str(write_fd), str(binary), *flags],
            pass_fds=(read_fd, write_fd), start_new_session=True,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except BaseException:
        send.close()
        receive.close()
        raise
    finally:
        os.close(read_fd)
        os.close(write_fd)
    return process, OnlineCDP(receive, send), flags


STARTUP_PROBE_JS = """(() => {
const videos=[...document.querySelectorAll('video')],v=videos[0],p=document.querySelector('#movie_player');
const menus=[...document.querySelectorAll('.ytp-menuitem')];
const rect=v?.getBoundingClientRect();
const quality=menus.filter(i=>/^quality$/i.test(i.querySelector('.ytp-menuitem-label')?.textContent.trim()??''));
const hd=menus.filter(i=>/^1080p(?:60)?(?:\\s|$)/i.test(i.textContent.trim())&&!/premium/i.test(i.textContent));
return {video_count:Math.min(videos.length,32),width:v?.videoWidth??0,height:v?.videoHeight??0,
 ready_state:v?.readyState??0,paused:v?.paused??null,muted:v?.muted??null,
 current_time:v?.currentTime??null,error:v?.error?.code??null,
 css_width:rect?.width??null,css_height:rect?.height??null,dpr:devicePixelRatio,
 viewport_width:innerWidth,viewport_height:innerHeight,
 consent_origin:['consent.youtube.com','consent.google.com'].includes(location.hostname),
 ad_showing:!!p?.classList.contains('ad-showing'),settings_count:Math.min(document.querySelectorAll('.ytp-settings-button').length,32),
 menu_count:Math.min(menus.length,32),quality_option_count:Math.min(quality.length,32),hd_option_count:Math.min(hd.length,32),
 consent_button_present:[...document.querySelectorAll('button,[role=button]')].some(b=>b.getClientRects().length>0&&getComputedStyle(b).visibility==='visible'&&!b.disabled&&['reject all','odmítnout vše','alle ablehnen'].includes(b.textContent.trim().toLowerCase()))};})()"""


def sanitized_startup_probe(probe):
    result = {}
    for key in ("video_count", "width", "height", "ready_state", "current_time", "error",
                "settings_count", "menu_count", "quality_option_count", "hd_option_count",
                "css_width", "css_height", "dpr", "viewport_width", "viewport_height"):
        value = probe.get(key)
        result[key] = value if type(value) in (int, float) and math.isfinite(value) and 0 <= value <= 1000000 else None
    for key in ("paused", "muted", "ad_showing", "consent_button_present", "consent_origin"):
        result[key] = probe.get(key) if type(probe.get(key)) is bool else None
    return result


def prepare_page(cdp, session, identity, url, evidence=None, navigate=True):
    evidence = evidence if evidence is not None else {}
    evidence["startup_step"] = "navigate_and_settle_content"
    if navigate:
        cdp.call("Page.enable", session=session)
        cdp.call("DOM.enable", session=session)
        cdp.media_session = session
        cdp.call("Media.enable", session=session)
        cdp.call("Page.navigate", {"url": url}, session)
    deadline, consent = time.monotonic() + 60, False
    consent_clicks = 0
    while time.monotonic() < deadline:
        ready = cdp.evaluate(session, """(() => {
const reject=[...document.querySelectorAll('button,[role=button]')].find(b=>b.getClientRects().length>0&&getComputedStyle(b).visibility==='visible'&&!b.disabled&&['reject all','odmítnout vše','alle ablehnen'].includes(b.textContent.trim().toLowerCase()));
if(reject){reject.click();return 'consent_rejected';}
const p=document.querySelector('#movie_player'),v=document.querySelector('video');
return v&&p&&v.readyState>=2&&v.videoWidth>0&&!p.classList.contains('ad-showing')&&typeof p.getVideoData==='function'&&p.getVideoData().video_id===new URL(location.href).searchParams.get('v')?'video_ready':'waiting';})()""", True)
        consent |= ready == "consent_rejected"
        if ready == "consent_rejected":
            consent_clicks += 1
            evidence["consent_clicks"] = min(evidence.get("consent_clicks", 0) + 1, 6)
            if consent_clicks >= 3:
                raise ValueError("Visible consent did not settle after bounded rejection")
            time.sleep(1)
        if ready == "video_ready":
            break
        time.sleep(0.25)
    else:
        raise TimeoutError("Consent, advertisements or public video did not settle before startup deadline")
    # Use ordinary site quality controls, once and before sampling. No altered
    # video CSS or network interception is used to manufacture a match.
    evidence["startup_step"] = "open_quality_menu"
    cdp.evaluate(session, "document.querySelector('.ytp-settings-button')?.click()", True)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if cdp.evaluate(session, """(() => {const item=[...document.querySelectorAll('.ytp-menuitem')].find(i=>/^quality$/i.test(i.querySelector('.ytp-menuitem-label')?.textContent.trim()??''));if(!item)return false;item.click();return true;})()""", True):
            break
        time.sleep(0.1)
    else:
        raise ValueError("Site quality control is unavailable")
    evidence["startup_step"] = "select_public_1080p"
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if cdp.evaluate(session, """(() => {const item=[...document.querySelectorAll('.ytp-menuitem')].find(i=>/^1080p(?:60)?(?:\\s|$)/i.test(i.textContent.trim())&&!/premium/i.test(i.textContent));if(!item)return false;item.click();return true;})()""", True):
            break
        time.sleep(0.1)
    else:
        raise ValueError("Public 1080p quality is unavailable")
    evidence["startup_step"] = "settle_actual_1080p_playback"
    cdp.call("Page.bringToFront", session=session)
    cdp.evaluate(session, """(() => {const p=document.querySelector('#movie_player'),v=document.querySelector('video');p?.unMute?.();p?.setVolume?.(100);p?.playVideo?.();v.muted=false;v.volume=1;return v.play();})()""", True)
    # Quality selection can change the element/source. Bind after it settles.
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if cdp.evaluate(session, """(() => {const v=document.querySelector('video');return !!v&&v.videoWidth===1920&&v.videoHeight===1080&&v.readyState>=3&&!v.paused;})()"""):
            break
        time.sleep(0.25)
    else:
        raise ValueError("Actual 1080p playback did not settle")
    evidence["startup_step"] = "complete"
    return consent


def bind_current_video(cdp, session):
    # Site navigation/quality changes may replace the video during warmup. Bind
    # once after warmup; from here through the measured interval it must remain
    # the exact same DOM element and selected stream.
    if not cdp.evaluate(session, INSTRUMENT_JS):
        raise ValueError("A unique current site video is unavailable")
    document = cdp.call("DOM.getDocument", session=session)["root"]["nodeId"]
    node = cdp.call("DOM.querySelector", {"nodeId": document, "selector": "video"}, session)["nodeId"]
    backend = cdp.call("DOM.describeNode", {"nodeId": node}, session)["node"]["backendNodeId"]
    return backend


def run(args):
    identity, url = public_video(args.url)
    if not (10 <= args.warmup <= 15 and args.seconds == 60
            and 1 <= args.width <= 2560 and 1 <= args.height <= 1600
            and 760 <= args.window_width <= 2560 and 600 <= args.window_height <= 1600):
        raise ValueError("Require 10–15s warm-up, 60s sample and bounded physical geometry")
    base = browser.ARTIFACTS / browser.VERSION
    verified = browser.verify_application(base)
    provenance = json.loads((browser.ARTIFACTS / "provenance.json").read_text())
    if verified["executable_sha256"] != provenance["executable_sha256"]:
        raise ValueError("Prepared pinned browser changed")
    output = browser.ROOT / "artifacts/youtube-browser-baseline"
    output.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="run-", dir=output))
    run_dir.chmod(0o700)
    result = {"schema": 1, "status": "incomplete", "public_video_id": identity,
              "browser": provenance, "harness_sha256": browser.sha256(Path(__file__)),
              "physical_video_width": args.width, "physical_video_height": args.height,
              "expected_video_codec": args.codec, "comparison": args.comparison, "warmup_seconds": args.warmup,
              "admission_only": args.admission_only,
              "physical_geometry_tolerance_pixels": 1.0 if args.comparison == "default-playback" else 0.1,
              "include_new_vt_services": args.include_new_vt_services, "samples": []}
    process, cdp, failure = None, None, None
    baseline = set(measure.process_table())
    footprint = measure.MacosFootprint() if args.macos_footprint else None
    phase = "browser_launch"
    try:
        process, cdp, flags = launch(base / browser.EXE_REL, run_dir / "profile", args.window_width, args.window_height)
        result["flags"] = [f if not f.startswith("--user-data-dir=") else
                           "--user-data-dir=<isolated-run-profile>" for f in flags]
        # Version data is from a pinned binary; no provider response enters it.
        version = cdp.call("Browser.getVersion")
        result["runtime_version"] = {k: version[k] for k in ("protocolVersion", "product", "revision")}
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            target = next((t for t in cdp.call("Target.getTargets")["targetInfos"] if t["type"] == "page"), None)
            if target:
                break
            time.sleep(0.05)
        else:
            raise TimeoutError("Native browser page startup deadline")
        session = cdp.call("Target.attachToTarget", {"targetId": target["targetId"], "flatten": True})["sessionId"]
        result["native_window"] = cdp.call("Browser.getWindowForTarget", {"targetId": target["targetId"]})
        phase = "site_startup_and_quality_selection"
        try:
            consent = prepare_page(cdp, session, identity, url, result)
        except Exception:
            try:
                result["startup_diagnostics"] = sanitized_startup_probe(cdp.evaluate(session, STARTUP_PROBE_JS))
                result["startup_decoder_diagnostics"] = cdp.association_diagnostics(-1, None)
            except Exception:
                result["startup_diagnostics_unavailable"] = True
            raise
        result["consent"] = "reject_all_clicked_before_sampling" if consent else "no_consent_click_required"
        time.sleep(args.warmup)
        # The initial site hydration may replace its video after the first
        # quality selection. One ordinary-control retry is allowed entirely
        # before admission, followed by the same full warmup. No measured state
        # is repaired and no repeated retry can mask an unstable interval.
        probe = sanitized_startup_probe(cdp.evaluate(session, STARTUP_PROBE_JS))
        result["post_warmup_startup_probe"] = probe
        if (probe["width"] != 1920 or probe["height"] != 1080 or probe["paused"] is not False
                or probe["consent_button_present"] is True):
            phase = "post_warmup_normal_controls_retry"
            result["startup_control_retries"] = 1
            consent_retry = prepare_page(cdp, session, identity, url, result, navigate=False)
            if consent_retry:
                result["consent"] = "reject_all_clicked_before_sampling"
            time.sleep(args.warmup)
        phase = "post_warmup_video_binding"
        backend = bind_current_video(cdp, session)
        phase = "initial_codec_hardware_geometry_admission"
        first = cdp.evaluate(session, SNAPSHOT_JS)
        # Record sanitized state before any strict association/track parsing so
        # a failed online attempt remains useful evidence, never a baseline.
        result["media_start"] = sanitized_snapshot(first)
        result["admission_diagnostics"] = cdp.association_diagnostics(backend, first.get("source_hash"))
        phase = "initial_player_association_admission"
        player, association = cdp.current_player(backend, first.get("source_hash"))
        result["decoder_association"] = association
        result["decoder_start_diagnostics"] = decoder_diagnostics(cdp.players[player])
        phase = "initial_selected_decoder_admission"
        decoder = decoder_evidence(cdp.players[player])
        result.update({"media_start": sanitized_snapshot(first), "decoder_start": decoder,
                       "decoder_association": association})
        phase = "initial_codec_hardware_geometry_admission"
        result["format_comparison"] = {"app_reference_video_codec": args.codec,
            "site_video_codec": decoder["video_codec"], "codec_matched": args.codec == decoder["video_codec"],
            "site_hardware_decoder": decoder["video_decoder"] == "VideoToolboxVideoDecoder" and decoder["platform_video_decoder"],
            "backend_isolation": False}
        qualify(first, decoder, identity, args.width, args.height, args.codec, args.seconds + 3, args.comparison)
        # A short preflight proves >=50fps before accepting a resource interval.
        phase = "forward_playback_preflight"
        preflight_started = time.monotonic()
        time.sleep(2)
        start_media = cdp.evaluate(session, SNAPSHOT_JS)
        result["preflight_media_end"] = sanitized_snapshot(start_media)
        preflight_elapsed = time.monotonic() - preflight_started
        preflight_frames, preflight_drops = qualify_online_interval(first, start_media, preflight_elapsed)
        result["playback_preflight"] = {"seconds": preflight_elapsed, "frame_delta": preflight_frames,
                                        "dropped_frame_delta": preflight_drops,
                                        "observed_fps": preflight_frames / preflight_elapsed}
        qualify(start_media, decoder, identity, args.width, args.height, args.codec, args.seconds + 1, args.comparison)
        if cdp.current_player(backend, start_media.get("source_hash"))[0] != player:
            raise ValueError("Selected player changed during preflight")
        epoch = cdp.epochs.get(player, 0)
        result["media_start"] = sanitized_snapshot(start_media)
        result["decoder_start"] = decoder_evidence(cdp.players[player])
        qualify(start_media, result["decoder_start"], identity, args.width, args.height, args.codec, args.seconds + 1, args.comparison)
        if args.admission_only:
            result["status"] = "admitted_playback_diagnostic_no_resource_interval"
            raise AdmissionComplete()
        result["cdp_processes_start"] = cdp.call("SystemInfo.getProcessInfo")["processInfo"]
        result["native_window"] = cdp.call("Browser.getWindowForTarget", {"targetId": target["targetId"]})
        table = measure.process_table()
        previous = measure.snapshot(process.pid, args.include_new_vt_services, baseline, table)
        result["process_inventory_start"] = browser.process_inventory(process.pid, previous, table, result["cdp_processes_start"])
        result["measurement_start_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        last = start = time.monotonic()
        exhausted_ticks = 0
        phase = "resource_sampling"
        for index in range(args.seconds):
            time.sleep(max(0, start + index + 1 - time.monotonic()))
            now = time.monotonic()
            current_table = measure.process_table()
            current = measure.snapshot(process.pid, args.include_new_vt_services, baseline, current_table)
            if process.poll() is not None:
                raise RuntimeError("Browser exited during measurement")
            row = {"rss_mib": sum(v[1] for v in current.values()) / 1024,
                   "cpu_one_core_percent": 100 * measure.cpu_delta(current, previous) / (now - last),
                   "processes": len(current), "elapsed_after_warmup_seconds": now - start,
                   **measure.host_other_cpu(current_table, table, set(current), now - last),
                   **measure.process_rss_audit(process.pid, current)}
            if footprint:
                row["macos_footprint"] = measure.footprint_audit(process.pid, current, footprint)
            # Consume Media events without issuing any browser/DOM command.
            # Keeping the pipe served prevents observer backpressure from
            # becoming part of the measured player workload.
            row["cdp_event_drain"] = cdp.drain_idle_events()
            result["samples"].append(row)
            exhausted_ticks = exhausted_ticks + 1 if row["cdp_event_drain"]["budget_exhausted"] else 0
            if exhausted_ticks >= 2:
                phase = "cdp_event_backpressure_admission"
                raise ValueError("Persistent CDP event backlog exceeds bounded idle drain")
            previous, table, last = current, current_table, now
        end_media = cdp.evaluate(session, SNAPSHOT_JS)
        result["media_end"] = sanitized_snapshot(end_media)
        result["end_admission_diagnostics"] = cdp.association_diagnostics(backend, end_media.get("source_hash"))
        phase = "interval_continuity_admission"
        end_player, _ = cdp.current_player(backend, end_media.get("source_hash"))
        end_decoder = decoder_evidence(cdp.players[end_player])
        result.update({"media_end": sanitized_snapshot(end_media), "decoder_end": end_decoder})
        elapsed = time.monotonic() - start
        qualify(end_media, end_decoder, identity, args.width, args.height, args.codec, 0, args.comparison)
        if (end_player != player or cdp.epochs.get(player, 0) != epoch or end_decoder != result["decoder_start"]
                or end_media.get("source_hash") != start_media.get("source_hash")):
            raise ValueError("Selected decoder or stream changed during measurement")
        frames, drops = qualify_online_interval(start_media, end_media, elapsed)
        result.update({"frame_delta": frames, "dropped_frame_delta": drops,
                       "media_counter_interval_seconds": elapsed, "observed_fps": frames / elapsed})
        result["cdp_processes_end"] = cdp.call("SystemInfo.getProcessInfo")["processInfo"]
        result["process_inventory_end"] = browser.process_inventory(process.pid, previous, table, result["cdp_processes_end"])
        for name in ("rss_mib", "cpu_one_core_percent", "host_other_cpu_one_core_percent"):
            result[name] = measure.statistics([row[name] for row in result["samples"]])
        if footprint:
            complete = [row["macos_footprint"]["aggregate_physical_footprint_mib"]
                        for row in result["samples"] if row["macos_footprint"]["complete"]]
            result["macos_footprint"] = {"complete_sample_count": len(complete),
                "all_samples_complete": len(complete) == args.seconds,
                "physical_footprint_mib": measure.statistics(complete) if len(complete) == args.seconds else None}
        # Drops are retained; failure cannot be promoted into a matching baseline.
        if drops:
            raise ValueError("Measured playback dropped video frames")
        result["status"] = ("completed_matched_online_playback_needs_external_review" if args.comparison == "matched-codec"
                            else "completed_default_site_playback_needs_external_review")
    except AdmissionComplete:
        pass
    except BaseException as error:
        failure = error
        result["failure_type"] = type(error).__name__
        result["failure_phase"] = phase
        if phase in ("post_warmup_video_binding", "post_warmup_normal_controls_retry"):
            try:
                result["startup_diagnostics"] = sanitized_startup_probe(cdp.evaluate(session, STARTUP_PROBE_JS))
                result["startup_decoder_diagnostics"] = cdp.association_diagnostics(-1, None)
            except Exception:
                result["startup_diagnostics_unavailable"] = True
        # Fixed harness errors only: never echo browser/provider error messages.
    finally:
        result["notes"] = ("Entire isolated browser tree, not incremental tab cost. Public logged-out YouTube; "
            "advertisements/consent settle before sampling, startup/extraction cost excluded. Audio codec/channels "
            "must match the app independently. DOM focus/geometry do not establish native scanout or audible "
            "hardware routing. Video counters are submitted/dropped frames, not compositor presentations. "
            "Shared RSS can double-count; footprint is a separate OS ledger, never added to RSS/GPU memory. "
            "New VT services are temporal attribution; preexisting/reparented/short-lived services can be missed. "
            "Network, WindowServer, display/power/thermals and unrelated work require external review. "
            "No signed URLs, cookies, provider responses or personal profile data are exported. "
            "default-playback permits the site's observed codec/software decoder: any difference from the app's "
            "selected format is a product-defaults comparison and cannot establish a native API advantage.")
        browser.finalize(run_dir, result, process, cdp)
    print(run_dir / "result.json")
    if failure:
        raise RuntimeError("Online browser admission or interval failed; inspect sanitized evidence") from None
    if result["status"] == "incomplete":
        raise RuntimeError("Online browser did not exit cleanly")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True, help="One public HTTPS youtube.com/watch?v=ID or youtu.be/ID")
    parser.add_argument("--codec", required=True, choices=sorted(CODECS), help="App reference codec (strictly required to match in matched-codec mode)")
    parser.add_argument("--comparison", choices=("matched-codec", "default-playback"), default="matched-codec",
                        help="Strict same-codec hardware admission, or actual site-selected codec/decoder at the same quality")
    parser.add_argument("--width", type=int, required=True, help="Matching physical video width, not window width")
    parser.add_argument("--height", type=int, required=True, help="Matching physical video height")
    parser.add_argument("--window-width", type=int, default=1320, help="Native browser window width for geometry calibration")
    parser.add_argument("--window-height", type=int, default=860, help="Native browser window height for geometry calibration")
    parser.add_argument("--warmup", type=int, default=10)
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--admission-only", action="store_true",
                        help="Finish after the 2 s throughput preflight; emits no performance comparison")
    parser.add_argument("--include-new-vt-services", action="store_true")
    parser.add_argument("--macos-footprint", action="store_true")
    args = parser.parse_args()
    def terminated(_signum, _frame):
        raise InterruptedError("Harness interrupted")
    previous = signal.signal(signal.SIGTERM, terminated)
    try:
        run(args)
    finally:
        signal.signal(signal.SIGTERM, previous)


if __name__ == "__main__":
    main()
