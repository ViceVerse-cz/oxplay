#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Plan, version, annotate and publish nightly/production releases.

A standard-library port of the semantic-release planner used by the release
workflow's model project: `plan` picks the next semantic version from the
commits since the last stable tag and records an immutable plan;
`apply-version` writes that version into the workspace manifests;
`commit-version` (production only) fast-forwards `main` to one release commit;
`publish` creates the GitHub release. Nothing is written remotely in a dry run:
`commit-version` then only commits in the local checkout and `publish` refuses.
"""
from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
PLAN = Path("target/release-plan.json")
NOTES = Path("target/release-notes.md")
CASK = Path("Casks/oxplay.rb")
CHANNELS = ("nightly", "production")
NUMBER = r"(?:0|[1-9][0-9]*)"
STABLE = re.compile(rf"v({NUMBER})\.({NUMBER})\.({NUMBER})")
VERSION = re.compile(rf"{NUMBER}\.{NUMBER}\.{NUMBER}(?:-nightly\.[0-9]{{8}}\.{NUMBER})?")
HEADER = re.compile(r"^(?P<type>[A-Za-z]+)(?:\((?P<scope>[^()\r\n]*)\))?(?P<breaking>!)?: (?P<subject>\S.*)$")
PULL_REQUEST = re.compile(r"\s+\(#(\d+)\)\s*$")
# conventional-changelog-conventionalcommits defaults: visible sections, in order.
SECTIONS = {"feat": "Features", "fix": "Bug Fixes", "perf": "Performance Improvements", "revert": "Reverts"}
HIDDEN = {"docs", "style", "chore", "refactor", "test", "build", "ci"}
BUMPS = {"major": 3, "minor": 2, "patch": 1, None: 0}
SEPARATOR, RECORD = "\x1f", "\x1e"


class ReleaseError(Exception):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ReleaseError(message)


def git(*arguments: str, cwd: Path = ROOT, check: bool = True) -> str:
    result = subprocess.run(["git", *arguments], cwd=cwd, stdin=subprocess.DEVNULL,
                            capture_output=True, text=True, timeout=120, check=False)
    if check and result.returncode:
        raise ReleaseError("git " + arguments[0] + " failed")
    return result.stdout if result.returncode == 0 else ""


# --- commit analysis -------------------------------------------------------

def parse_commit(record: str) -> dict:
    sha, name, email, subject, body = (record.split(SEPARATOR) + [""] * 5)[:5]
    commit = {"hash": sha.strip(), "author": {"name": name, "email": email},
              "subject": subject.strip(), "body": body.strip()}
    match = PULL_REQUEST.search(commit["subject"])
    commit["pull_request"] = match[1] if match else None
    header = PULL_REQUEST.sub("", commit["subject"])
    parsed = HEADER.match(header)
    breaking = re.search(r"^BREAKING[ -]CHANGE: ", commit["body"], re.MULTILINE) is not None
    if parsed:
        commit.update(type=parsed["type"].lower(), scope=parsed["scope"] or None,
                      title=parsed["subject"], breaking=breaking or bool(parsed["breaking"]))
    else:
        commit.update(type=None, scope=None, title=header, breaking=breaking)
    return commit


def commits(revision_range: str, cwd: Path = ROOT) -> list[dict]:
    """Non-merge commits, newest first; the release bot's own commits are ignored."""
    output = git("log", "--no-merges", f"--format=%H{SEPARATOR}%an{SEPARATOR}%ae{SEPARATOR}%s{SEPARATOR}%b{RECORD}",
                 revision_range, cwd=cwd)
    parsed = [parse_commit(item.strip("\n")) for item in output.split(RECORD) if item.strip()]
    return [item for item in parsed if not (item["type"] == "chore" and item["scope"] == "release")]


def bump(commit: dict) -> str | None:
    """semantic-release's conventionalcommits rules, plus one adaptation:
    a subject that is not a conventional header still counts as a patch, so
    plain-English history keeps producing releases."""
    if commit["breaking"]:
        return "major"
    if commit["type"] == "feat":
        return "minor"
    if commit["type"] in ("fix", "perf", "revert") or commit["type"] is None:
        return "patch"
    return None


def release_type(items: list[dict]) -> str | None:
    return max((bump(item) for item in items), key=BUMPS.__getitem__, default=None)


def increment(version: str, kind: str) -> str:
    major, minor, patch = (int(part) for part in version.split("."))
    if kind == "major":
        return f"{major + 1}.0.0"
    if kind == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def stable_tags(cwd: Path = ROOT) -> list[tuple[tuple[int, int, int], str]]:
    """Reachable vX.Y.Z tags only; nightly/dev tags never become the last release."""
    tags = []
    for tag in git("tag", "--merged", "HEAD", "--list", "v*", cwd=cwd).split():
        if match := STABLE.fullmatch(tag):
            tags.append(((int(match[1]), int(match[2]), int(match[3])), tag))
    return sorted(tags)


def latest_release_tag(ref: str = "HEAD", cwd: Path = ROOT) -> str | None:
    """Nearest earlier release tag of any channel (git describe, like notes.mjs)."""
    if git("describe", "--tags", "--match", "v[0-9]*", "--exact-match", ref, cwd=cwd, check=False):
        ref = ref + "~1"
    return git("describe", "--tags", "--match", "v[0-9]*", "--abbrev=0", ref, cwd=cwd, check=False).strip() or None


def workspace_version(source: str) -> str:
    version = tomllib.loads(source)["workspace"]["package"]["version"]
    require(isinstance(version, str) and re.fullmatch(rf"{NUMBER}\.{NUMBER}\.{NUMBER}", version) is not None,
            "The committed workspace version must be a plain X.Y.Z version")
    return version


# --- release notes (port of notes.mjs) ---------------------------------------

def author_key(author: dict) -> str | None:
    value = (author.get("email") or author.get("name") or "").strip().lower()
    return value or None


def author_label(author: dict) -> str | None:
    email = (author.get("email") or "").strip()
    login = re.fullmatch(r"(?:\d+\+)?([^@]+)@users\.noreply\.github\.com", email, re.IGNORECASE)
    if login:
        return f"[@{login[1]}](https://github.com/{login[1]})"
    name = re.sub(r"[\r\n]+", " ", author.get("name") or "").strip()
    return name or None


def previous_authors(ref: str | None, cwd: Path = ROOT) -> set[str]:
    if not ref:
        return set()
    output = git("log", ref, "--format=%an\t%ae", cwd=cwd, check=False)
    keys = set()
    for line in output.splitlines():
        name, _, email = line.partition("\t")
        if key := author_key({"name": name, "email": email}):
            keys.add(key)
    return keys


def markdown_line(commit: dict, repository: str) -> str:
    text = commit["title"].replace("\n", " ")
    line = "* " + (f"**{commit['scope']}:** " if commit["scope"] else "") + text
    if label := author_label(commit["author"]):
        line += f" by {label}"
    if commit["pull_request"]:
        line += f" in [#{commit['pull_request']}](https://github.com/{repository}/pull/{commit['pull_request']})"
    return line + f" ([{commit['hash'][:7]}](https://github.com/{repository}/commit/{commit['hash']}))"


def render_notes(version: str, previous: str | None, items: list[dict], repository: str,
                 known_authors: set[str], date: str) -> str:
    tag = f"v{version}"
    title = f"[{version}](https://github.com/{repository}/compare/{previous}...{tag})" if previous else version
    lines = [f"## {title} ({date})"]
    breaking = [item for item in items if item["breaking"]]
    if breaking:
        lines += ["", "### ⚠ BREAKING CHANGES", ""]
        lines += [markdown_line(item, repository) for item in breaking]
    groups = [(heading, [item for item in items if item["type"] == kind]) for kind, heading in SECTIONS.items()]
    # Adaptation: this history is mostly plain English; keep it visible.
    groups.append(("Changes", [item for item in items if item["type"] is None]))
    for heading, members in groups:
        if members:
            lines += ["", f"### {heading}", ""]
            lines += [markdown_line(item, repository) for item in members]
    contributors, seen = [], set()
    for item in reversed(items):  # oldest first: the actual first contribution
        key, label = author_key(item["author"]), author_label(item["author"])
        if (not item["pull_request"] or not key or not label or key in known_authors
                or key in seen or "[bot]" in label):
            continue
        seen.add(key)
        url = f"https://github.com/{repository}/pull/{item['pull_request']}"
        contributors.append(f"* {label} made their first contribution in [#{item['pull_request']}]({url})")
    if contributors:
        lines += ["", "## New Contributors", ""] + contributors
    if len(lines) == 1:
        lines += ["", "No user-facing changes since the previous release."]
    return "\n".join(lines) + "\n"


def install_notes(channel: str, version: str, macos_signed: bool) -> str:
    kind = "nightly prerelease" if channel == "nightly" else "release"
    mac = ("The macOS app is Developer ID signed and notarized." if macos_signed else
           "The macOS app is ad-hoc signed only (not notarized); after unzipping run "
           "`xattr -dr com.apple.quarantine Oxplay.app`.")
    return f"""
## Downloads

This {kind} ({version}) was built by CI from the tagged commit with the pinned
Rust toolchain and `--locked` dependencies.

| Platform | Assets |
| --- | --- |
| macOS Apple Silicon | `oxplay-v{version}-macOS-ARM64.zip` (+ `.inventory.json`), Homebrew cask `oxplay` |
| Windows x86_64 | `oxplay-v{version}-Windows-X64-Setup.exe` (per-user installer) or `-Windows-X64.zip` (portable) |
| Linux x86_64 | `.deb` (Ubuntu 24.04), `.rpm` (Fedora), `.pkg.tar.zst` (Arch), `-Linux-X64.AppImage`, `-Linux-X64.tar.gz` |
| Source | `oxplay-v{version}-source.tar.gz` with `release.json` |

{mac} Windows and Linux builds are unsigned. yt-dlp and Deno are bundled at
pinned, SHA-256-verified versions. Every package includes the private native
libmpv/libplacebo stack: Metal on macOS, Vulkan on Linux, and DX12 UI with
D3D11 video on Windows. Native media source and dependency records accompany
the binaries. Oxplay is experimental and
not release-qualified; see `docs/packaging.md` and `docs/licensing.md` in the
source for the open portability, notice and corresponding-source items.

The performance audit found low idle CPU usage and lower memory use than a
minimal Chrome video player, but playback CPU remains above the target.
An advantage over the YouTube website has not been established. See
`docs/performance-playback-2026-10-01.md` in the source archive for measurements
and the limits of the comparisons. Linux and Windows hardware playback need
verification on physical devices.

`SHA256SUMS.txt` covers every attached asset. A checksum only proves integrity
when obtained through a trusted channel; it is not a code signature.
"""


# --- manifests (port of version.py) ----------------------------------------

def apply_version(version: str, root: Path = ROOT) -> None:
    """Update only workspace-inherited package versions; keep formatting and pins."""
    require(VERSION.fullmatch(version) is not None, "Expected a stable or nightly semantic version")
    manifest = root / "Cargo.toml"
    source = manifest.read_text(encoding="utf-8")
    workspace = tomllib.loads(source)["workspace"]
    previous = workspace["package"]["version"]
    names = set()
    for member in workspace["members"]:
        package = tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))["package"]
        if package.get("version") == {"workspace": True}:
            names.add(package["name"])
    require("oxplay" in names, "The application must inherit the workspace version")
    updated, count = re.subn(r'(\[workspace\.package\]\s*\nversion\s*=\s*")[^"]+("\s*\n)',
                             lambda match: match[1] + version + match[2], source, count=1)
    require(count == 1, "Cannot locate the workspace version")
    lock = root / "Cargo.lock"
    sections = lock.read_text(encoding="utf-8").split("[[package]]")
    changed = 0
    for index in range(1, len(sections)):
        section = sections[index]
        package = tomllib.loads(section)
        if package["name"] in names and "source" not in package:
            require(package["version"] == previous, "Unexpected local lockfile version")
            section = re.sub(r'(?m)^version = "[^"]+"$', f'version = "{version}"', section, count=1)
            changed += 1
        # Cargo disambiguates duplicate names as "name version" in dependency lists.
        for name in names:
            section = section.replace(f'"{name} {previous}"', f'"{name} {version}"')
        sections[index] = section
    require(changed == len(names), "Every workspace-versioned package must be locked")
    manifest.write_text(updated, encoding="utf-8")
    lock.write_text("[[package]]".join(sections), encoding="utf-8")


# --- plan --------------------------------------------------------------------

def plan(channel: str, *, dry_run: bool, ref_name: str, run_number: str, repository: str,
         today: dt.date, cwd: Path = ROOT) -> dict | None:
    require(channel in CHANNELS, "Choose nightly or production")
    require(dry_run or ref_name == "main", "Publish from main only; use dry_run on other branches")
    require(re.fullmatch(r"[1-9][0-9]{0,9}", run_number) is not None, "Invalid workflow run number")
    require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository) is not None, "Invalid repository")
    head = git("rev-parse", "--verify", "HEAD^{commit}", cwd=cwd).strip()
    tags = stable_tags(cwd)
    last = tags[-1][1] if tags else None
    items = commits(f"{last}..HEAD" if last else "HEAD", cwd)
    kind = release_type(items)
    if kind is None:
        return None
    if last:
        stable = increment(last[1:], kind)
    else:  # First stable release: the committed workspace version, not 1.0.0.
        stable = workspace_version(git("show", "HEAD:Cargo.toml", cwd=cwd))
    date = today.strftime("%Y%m%d")
    version = f"{stable}-nightly.{date}.{run_number}" if channel == "nightly" else stable
    tag = f"v{version}"
    require(tag not in git("tag", "--list", tag, cwd=cwd).split(), "The planned tag already exists")
    if channel == "production":
        previous, noted = last, items
    else:
        # Nightly isolation: only what changed since the nearest release of any channel.
        previous = latest_release_tag("HEAD", cwd)
        noted = commits(f"{previous}..HEAD", cwd) if previous else items
    notes = render_notes(version, previous, noted, repository, previous_authors(previous, cwd), today.isoformat())
    return {"schema": 1, "channel": channel, "dry_run": dry_run, "version": version,
            "stable_version": stable, "tag": tag, "git_head": head, "previous_tag": previous,
            "last_stable_tag": last, "release_type": kind, "notes": notes, "repository": repository}


def write_outputs(path: str | None, values: dict[str, str]) -> None:
    if not path:
        return
    with open(path, "a", encoding="utf-8") as output:
        for key, value in values.items():
            require("\n" not in value and "\r" not in value, "Workflow outputs must be single-line")
            output.write(f"{key}={value}\n")


def load_plan(channel: str) -> dict:
    data = json.loads(PLAN.read_text(encoding="utf-8"))
    require(data.get("schema") == 1 and data.get("channel") == channel, "Release channel changed after planning")
    require(VERSION.fullmatch(data["version"]) is not None and data["tag"] == "v" + data["version"],
            "Malformed release plan")
    return data


# --- publish -------------------------------------------------------------------

def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def update_cask(data: dict, assets: Path, cask: Path = CASK) -> None:
    archive = assets / f"oxplay-{data['tag']}-macOS-ARM64.zip"
    require(archive.is_file(), "The macOS release archive is missing")
    source = cask.read_text(encoding="utf-8")
    require(re.search(r'^  version "[^"]+"$', source, re.MULTILINE) is not None, "Cannot locate the cask version")
    require(re.search(r'^  sha256 "[0-9a-f]{64}"$', source, re.MULTILINE) is not None, "Cannot locate the cask checksum")
    source = re.sub(r'(?m)^(  version ")[^"]+("$)', lambda m: m[1] + data["version"] + m[2], source)
    source = re.sub(r'(?m)^(  sha256 ")[0-9a-f]{64}("$)', lambda m: m[1] + sha256(archive) + m[2], source)
    cask.write_text(source, encoding="utf-8")


def gh(*arguments: str, fields: dict | None = None) -> dict | list | str:
    """`gh api` with a JSON body on stdin; GH_TOKEN never enters argv or logs."""
    command = ["gh", "api", *arguments]
    if fields is not None:
        command += ["--input", "-"]
    result = subprocess.run(command, input=json.dumps(fields) if fields is not None else None,
                            capture_output=True, text=True, timeout=120, check=False)
    if result.returncode:
        raise ReleaseError(f"GitHub API request failed: {' '.join(arguments[:3])}: {result.stderr.strip()[:500]}")
    return json.loads(result.stdout) if result.stdout.strip() else ""


def commit_version(data: dict, assets: Path) -> str:
    """One release commit on top of the planned head; main only fast-forwards.

    The ref update is `force: false`, so a branch that moved while packages were
    building rejects the release instead of silently including unbuilt commits.
    """
    repository = data["repository"]
    update_cask(data, assets)
    base = gh(f"repos/{repository}/git/commits/{data['git_head']}")
    entries = []
    for path in ("Cargo.toml", "Cargo.lock", CASK.as_posix()):
        blob = gh(f"repos/{repository}/git/blobs", "--method", "POST",
                  fields={"content": base64.b64encode(Path(path).read_bytes()).decode(), "encoding": "base64"})
        entries.append({"path": path, "mode": "100644", "type": "blob", "sha": blob["sha"]})
    tree = gh(f"repos/{repository}/git/trees", "--method", "POST",
              fields={"base_tree": base["tree"]["sha"], "tree": entries})
    commit = gh(f"repos/{repository}/git/commits", "--method", "POST",
                fields={"message": f"chore(release): {data['version']} [skip ci]", "tree": tree["sha"],
                        "parents": [data["git_head"]]})
    gh(f"repos/{repository}/git/refs/heads/main", "--method", "PATCH", fields={"sha": commit["sha"], "force": False})
    return commit["sha"]


def commit_version_locally(data: dict, assets: Path, cwd: Path = Path(".")) -> str:
    """Dry run: the same release commit, created only in the local checkout."""
    update_cask(data, assets, cwd / CASK)
    identity = ["-c", "user.name=github-actions[bot]",
                "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com", "-c", "commit.gpgsign=false"]
    git(*identity, "commit", "--quiet", "--no-verify", "-m", f"chore(release): {data['version']} [skip ci]",
        "--", "Cargo.toml", "Cargo.lock", CASK.as_posix(), cwd=cwd)
    return git("rev-parse", "HEAD", cwd=cwd).strip()


def run(*command: str) -> None:
    subprocess.run(command, check=True, timeout=1800)


def publish(data: dict, assets: Path, target: str, notes: Path) -> None:
    files = sorted(str(path) for path in assets.iterdir() if path.is_file())
    require(any(name.endswith("SHA256SUMS.txt") for name in files), "Checksums must be generated before publishing")
    title = f"Oxplay {data['version']}"
    # Create as a draft so the release only appears once every asset uploaded.
    run("gh", "release", "create", data["tag"], *files, "--target", target, "--title", title,
        "--notes-file", str(notes), "--draft", *(["--prerelease", "--latest=false"]
                                                 if data["channel"] == "nightly" else ["--latest"]))
    if data["channel"] == "nightly":
        run("gh", "release", "edit", data["tag"], "--draft=false", "--prerelease", "--latest=false")
        # Nightlies never commit versions; only the cask follows the newest build.
        update_cask(data, assets)
        current = git("rev-parse", f"origin/main:{CASK.as_posix()}").strip()
        gh(f"repos/{data['repository']}/contents/{CASK.as_posix()}", "--method", "PUT",
           fields={"message": f"chore(release): update Homebrew cask for {data['version']} [skip ci]",
                   "content": base64.b64encode(CASK.read_bytes()).decode(), "sha": current, "branch": "main"})
    else:
        run("gh", "release", "edit", data["tag"], "--draft=false", "--latest")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("plan", "notes", "commit-version", "publish"):
        command = commands.add_parser(name)
        command.add_argument("--channel", choices=CHANNELS, default=os.environ.get("RELEASE_CHANNEL"))
        command.add_argument("--dry-run", action="store_true", default=os.environ.get("DRY_RUN") == "true")
        if name != "plan":
            command.add_argument("--assets", type=Path, default=Path("release-assets"))
        if name == "publish":
            command.add_argument("--target", required=True, help="Commit the release tag points at")
    version = commands.add_parser("apply-version")
    version.add_argument("version")
    args = parser.parse_args()
    try:
        if args.command == "apply-version":
            apply_version(args.version)
            print(f"Workspace manifests set to {args.version}")
            return 0
        require(args.channel in CHANNELS, "Choose --channel nightly or production")
        if args.command == "plan":
            result = plan(args.channel, dry_run=args.dry_run, ref_name=os.environ.get("GITHUB_REF_NAME", ""),
                          run_number=os.environ.get("GITHUB_RUN_NUMBER", "1"),
                          repository=os.environ.get("GITHUB_REPOSITORY", "ViceVerse-cz/oxplay"),
                          today=dt.datetime.now(dt.timezone.utc).date())
            write_outputs(os.environ.get("GITHUB_OUTPUT"), {"release": str(result is not None).lower()})
            if result is None:
                print("No releasable commits since the last stable release.")
                return 0
            PLAN.parent.mkdir(parents=True, exist_ok=True)
            PLAN.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            write_outputs(os.environ.get("GITHUB_OUTPUT"), {name: result[name] for name in
                                                            ("version", "tag", "stable_version", "git_head")})
            print(f"Planned {result['channel']} {result['version']} ({result['release_type']}) at {result['git_head']}")
            return 0
        data = load_plan(args.channel)
        if args.command == "notes":
            signed = os.environ.get("MACOS_SIGNED") == "true"
            NOTES.parent.mkdir(parents=True, exist_ok=True)
            NOTES.write_text(data["notes"] + install_notes(data["channel"], data["version"], signed), encoding="utf-8")
            print(NOTES)
            return 0
        dry_run = args.dry_run or data["dry_run"]
        if args.command == "commit-version":
            require(data["channel"] == "production", "Nightlies never commit versions")
            if dry_run:
                sha = commit_version_locally(data, args.assets)
            else:
                require(os.environ.get("GITHUB_REF_NAME") == "main", "Publish from main only")
                sha = commit_version(data, args.assets)
            write_outputs(os.environ.get("GITHUB_OUTPUT"), {"commit": sha})
            print(f"Release commit {sha}{' (local only: dry run)' if dry_run else ''}")
            return 0
        require(not dry_run, "Dry runs never write to the repository")
        require(os.environ.get("GITHUB_REF_NAME") == "main", "Publish from main only")
        require(re.fullmatch(r"[0-9a-f]{40}", args.target) is not None, "Invalid release target")
        publish(data, args.assets, args.target, NOTES)
        return 0
    except (ReleaseError, OSError, KeyError, ValueError, subprocess.SubprocessError) as error:
        print(f"Release stopped: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
