// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit guest download of one public video through the supervised helper.
//!
//! The helper runs with the same isolation, anonymous access and rate-limit
//! cooldown as stream resolution, but outside the single playback extraction
//! slot: a long download must never delay opening a video. Callers bound how
//! many downloads run at once. Only machine-readable marker lines are parsed;
//! stderr is kept as a short tail for error classification and never shown.
use crate::{RateLimit, YtDlp, classify_failure, supervisor};
use oxplay_core::{OperationContext, ProviderError, VideoId};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Containers the helper may produce and the local player accepts.
pub const MEDIA_EXTENSIONS: &[&str] = &["mp4", "mkv", "webm", "m4v", "mov"];
/// Artwork the helper may write next to the media file (never converted).
pub const THUMBNAIL_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];
const LINE_LIMIT: usize = 16 * 1024;
const STDERR_TAIL: usize = 64 * 1024;
/// A download that makes no progress output for this long is stopped.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Hard ceiling for one download, including merging.
const OVERALL_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
const PLAN: &str = "OXPPLAN ";
const PROGRESS: &str = "OXPPROG ";
const DONE: &str = "OXPDONE ";

/// One explicit download into an empty, caller-owned staging directory.
pub struct Request<'a> {
    pub id: &'a VideoId,
    /// The user's quality ceiling (144–2160).
    pub max_height: u16,
    /// An explicit, reviewed FFmpeg executable used only to merge separate
    /// video and audio streams. `None` selects a single-file format instead.
    pub merger: Option<&'a Path>,
    pub staging: &'a Path,
}

/// Format selection for a download. With a merger, the best separate video
/// (at most `max_height`) plus the best audio are merged into MP4 when the
/// codecs allow it, otherwise Matroska. Without one, only a progressive file
/// that already contains both streams is accepted; YouTube usually offers
/// that only at 360p, so quality is limited rather than silently exceeded.
pub fn selection(max_height: u16, merger: Option<&Path>) -> Result<Vec<String>, ProviderError> {
    if !(144..=2160).contains(&max_height) {
        return Err(ProviderError::InvalidInput);
    }
    let h = max_height;
    let progressive = format!("best[height<={h}][protocol=https][vcodec!=none][acodec!=none]");
    let mut args = vec!["--format-sort".into(), "height,fps,vcodec:h264".into()];
    match merger {
        Some(path) => {
            let path = path.to_str().ok_or(ProviderError::InvalidInput)?;
            if !Path::new(path).is_absolute() {
                return Err(ProviderError::InvalidInput);
            }
            args.extend([
                "--format".into(),
                format!(
                    "bestvideo[height<={h}][protocol=https]+bestaudio[protocol=https]/{progressive}"
                ),
                "--merge-output-format".into(),
                "mp4/mkv".into(),
                "--ffmpeg-location".into(),
                path.into(),
            ]);
        }
        None => args.extend([
            "--format".into(),
            progressive,
            // No post-processing helper is available or searched for.
            "--fixup".into(),
            "never".into(),
        ]),
    }
    Ok(args)
}

/// Metadata the helper reported for the finished file. Strings are untrusted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    pub id: Option<String>,
    pub title: Option<String>,
    pub channel: Option<String>,
    pub duration: Option<Duration>,
    pub height: Option<u32>,
    pub ext: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RawProgress {
    pub finished: bool,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub estimate: Option<u64>,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    /// The helper's format identifier for the stream being fetched.
    pub part: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// Selected format: number of streams to fetch and the expected size.
    Plan {
        parts: usize,
        total: Option<u64>,
    },
    Progress(RawProgress),
    Done(Metadata),
    Other,
}

fn number(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}
fn bytes(value: &str) -> Option<u64> {
    number(value).map(|v| v.min(u64::MAX as f64) as u64)
}
fn text(value: &Value, key: &str, limit: usize) -> Option<String> {
    let value: String = value
        .get(key)?
        .as_str()?
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Parse one helper stdout line produced by the templates in [`arguments`].
pub fn parse_line(line: &str) -> Line {
    if let Some(rest) = line.strip_prefix(PLAN) {
        let mut fields = rest.split(' ');
        let parts = fields
            .next()
            .filter(|id| !id.is_empty())
            .map_or(1, |id| id.split('+').count().clamp(1, 4));
        return Line::Plan {
            parts,
            total: fields.next().and_then(bytes).filter(|n| *n > 0),
        };
    }
    if let Some(rest) = line.strip_prefix(PROGRESS) {
        let fields: Vec<&str> = rest.splitn(7, ' ').collect();
        let [status, downloaded, total, estimate, speed, eta, part] = fields[..] else {
            return Line::Other;
        };
        let finished = match status {
            "downloading" => false,
            "finished" => true,
            _ => return Line::Other,
        };
        let Some(downloaded) = bytes(downloaded) else {
            return Line::Other;
        };
        return Line::Progress(RawProgress {
            finished,
            downloaded,
            total: bytes(total).filter(|n| *n > 0),
            estimate: bytes(estimate).filter(|n| *n > 0),
            speed: number(speed),
            eta: bytes(eta),
            part: part.chars().take(64).collect(),
        });
    }
    if let Some(rest) = line.strip_prefix(DONE) {
        let Ok(value) = serde_json::from_str::<Value>(rest) else {
            return Line::Other;
        };
        if !value.is_object() {
            return Line::Other;
        }
        return Line::Done(Metadata {
            id: text(&value, "id", 64),
            title: text(&value, "title", 500),
            channel: text(&value, "channel", 200).or_else(|| text(&value, "uploader", 200)),
            duration: value
                .get("duration")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 31_536_000.0)
                .map(Duration::from_secs_f64),
            height: value
                .get("height")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| *v > 0 && *v <= 8640),
            ext: text(&value, "ext", 8),
        });
    }
    Line::Other
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Downloading,
    /// All streams arrived; merging/moving is in progress.
    Finishing,
}

/// Overall progress across every stream of one download.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// 0.0–1.0, or `None` while no size is known.
    pub fraction: Option<f32>,
    pub downloaded: u64,
    pub total: Option<u64>,
    /// Bytes per second for the current stream.
    pub speed: Option<f64>,
    /// Seconds remaining, when it can be estimated for the whole download.
    pub eta: Option<u64>,
    pub stage: Stage,
}

/// Combine per-stream helper progress into one bar. A merged download fetches
/// video and then audio, each reporting 0–100 %; the plan's expected size
/// weights them. Without a size, each stream counts equally.
#[derive(Default)]
pub struct Tracker {
    parts: usize,
    planned: Option<u64>,
    current: Option<String>,
    current_finished: bool,
    current_downloaded: u64,
    current_total: Option<u64>,
    completed_parts: usize,
    completed_bytes: u64,
}
impl Tracker {
    pub fn apply(&mut self, line: &Line) -> Option<Progress> {
        let raw = match line {
            Line::Plan { parts, total } => {
                self.parts = *parts;
                self.planned = *total;
                return None;
            }
            Line::Progress(raw) => raw,
            _ => return None,
        };
        if self.current.as_deref() != Some(raw.part.as_str()) {
            if self.current.is_some() && !self.current_finished {
                self.completed_parts += 1;
                self.completed_bytes += self.current_downloaded;
            }
            self.current = Some(raw.part.clone());
            self.current_finished = false;
            self.current_downloaded = 0;
            self.current_total = None;
        }
        let parts = self.parts.max(1);
        if raw.finished {
            if !self.current_finished {
                self.current_finished = true;
                self.completed_parts += 1;
                self.completed_bytes += raw.downloaded;
            }
            self.current_downloaded = 0;
            self.current_total = None;
        } else if !self.current_finished {
            self.current_downloaded = raw.downloaded;
            self.current_total = raw.total.or(raw.estimate);
        }
        let finishing = self.completed_parts >= parts;
        let downloaded = self.completed_bytes + self.current_downloaded;
        let total = self
            .planned
            .or_else(|| (parts == 1).then_some(self.current_total).flatten())
            .map(|total| total.max(downloaded));
        let fraction = if finishing {
            Some(1.0)
        } else if let Some(total) = total {
            Some((downloaded as f64 / total as f64).min(0.99) as f32)
        } else {
            let current = self
                .current_total
                .map_or(0.0, |t| self.current_downloaded as f64 / t.max(1) as f64);
            Some(
                (((self.completed_parts as f64 + current.min(1.0)) / parts as f64).min(0.99))
                    as f32,
            )
        };
        let speed = (!raw.finished).then_some(raw.speed).flatten();
        let eta = match (total, speed) {
            (_, _) if finishing || raw.finished => None,
            (Some(total), Some(speed)) if speed > 0.0 && self.planned.is_some() => {
                Some((total.saturating_sub(downloaded) as f64 / speed).ceil() as u64)
            }
            // The helper's own estimate covers only the current stream.
            _ if self.completed_parts + 1 == parts => raw.eta,
            _ => None,
        };
        Some(Progress {
            fraction,
            downloaded,
            total,
            speed,
            eta,
            stage: if finishing {
                Stage::Finishing
            } else {
                Stage::Downloading
            },
        })
    }
}

/// The finished files in the staging directory, not yet moved by the caller.
#[derive(Debug)]
pub struct Outcome {
    pub media: PathBuf,
    pub thumbnail: Option<PathBuf>,
    pub metadata: Metadata,
    /// True when separate streams were merged (the requested ceiling applied);
    /// false for a single progressive file.
    pub merged: bool,
}

fn regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Locate the helper's output inside `staging`. Only `<id>.<ext>` names with
/// allowed extensions are accepted; symlinks and directories are ignored.
pub fn staged_files(
    staging: &Path,
    id: &VideoId,
    ext: Option<&str>,
) -> Result<(PathBuf, Option<PathBuf>), ProviderError> {
    let media = ext
        .filter(|ext| MEDIA_EXTENSIONS.contains(ext))
        .map(|ext| staging.join(format!("{}.{ext}", id.as_str())))
        .filter(|path| regular_file(path))
        .or_else(|| {
            MEDIA_EXTENSIONS
                .iter()
                .map(|ext| staging.join(format!("{}.{ext}", id.as_str())))
                .find(|path| regular_file(path))
        })
        .ok_or(ProviderError::MalformedOutput)?;
    let thumbnail = THUMBNAIL_EXTENSIONS
        .iter()
        .map(|ext| staging.join(format!("{}.{ext}", id.as_str())))
        .find(|path| regular_file(path));
    Ok((media, thumbnail))
}

impl YtDlp {
    fn download_arguments(&self, request: &Request) -> Result<Vec<String>, ProviderError> {
        let staging = request
            .staging
            .to_str()
            .ok_or(ProviderError::InvalidInput)?;
        if !request.staging.is_absolute() {
            return Err(ProviderError::InvalidInput);
        }
        let mut args: Vec<String> = [
            "--ignore-config",
            "--no-config-locations",
            "--no-plugin-dirs",
            "--no-cache-dir",
            "--no-js-runtimes",
            "--no-remote-components",
            "--no-mark-watched",
            "--no-cookies-from-browser",
            "--proxy",
            "",
            "--no-playlist",
            // A live stream never finishes; it is skipped and reported instead.
            "--match-filters",
            "!is_live",
            "--socket-timeout",
            "30",
            "--retries",
            "3",
            "--fragment-retries",
            "3",
            "--extractor-retries",
            "0",
            "--no-mtime",
            "--quiet",
            "--no-warnings",
            "--progress",
            "--newline",
            "--progress-delta",
            "0.5",
            "--progress-template",
            "download:OXPPROG %(progress.status)s %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s %(progress.speed)s %(progress.eta)s %(info.format_id)s",
            "--no-simulate",
            "--print",
            "before_dl:OXPPLAN %(format_id)s %(filesize,filesize_approx)s",
            "--print",
            "after_move:OXPDONE %(.{id,title,channel,uploader,duration,height,ext})j",
            "--write-thumbnail",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        args.extend([
            "--paths".into(),
            staging.to_owned(),
            "--output".into(),
            "%(id)s.%(ext)s".into(),
        ]);
        if let Some(deno) = &self.deno {
            args.push("--js-runtimes".into());
            args.push(format!("deno:{}", deno.display()));
        }
        args.extend(selection(request.max_height, request.merger)?);
        args.extend(["--".into(), request.id.watch_url()]);
        Ok(args)
    }

    /// Download one public video anonymously. Blocking: call on a worker
    /// thread. Progress is reported as the helper prints it (about twice a
    /// second); the caller coalesces UI updates. Cancelling `operation` kills
    /// the helper tree; the caller then removes `staging`.
    pub fn download(
        &self,
        request: &Request,
        operation: &OperationContext,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Outcome, ProviderError> {
        if operation.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let args = self.download_arguments(request)?;
        if self
            .cooldown
            .lock()
            .map_err(|_| ProviderError::ExtractorFailed)?
            .is_some_and(RateLimit::active)
        {
            return Err(ProviderError::RateLimited);
        }
        let mut tracker = Tracker::default();
        let mut done = None;
        let mut parts = 1;
        let output = supervisor::run_streaming(
            &self.binary,
            &args,
            operation,
            supervisor::StreamLimits {
                overall: OVERALL_TIMEOUT,
                idle: IDLE_TIMEOUT,
                line: LINE_LIMIT,
                stderr_tail: STDERR_TAIL,
            },
            &|| false,
            &mut |line| {
                let line = parse_line(line);
                match &line {
                    Line::Done(metadata) => done = Some(metadata.clone()),
                    Line::Plan { parts: count, .. } => parts = *count,
                    _ => {}
                }
                if let Some(progress) = tracker.apply(&line) {
                    on_progress(progress);
                }
            },
        )?;
        if !output.success {
            let error = classify_failure(&output.stderr);
            if error == ProviderError::RateLimited {
                *self
                    .cooldown
                    .lock()
                    .map_err(|_| ProviderError::ExtractorFailed)? =
                    Some(RateLimit::after(Duration::from_secs(60)));
            }
            return Err(error);
        }
        // Exit 0 without a finished file: filtered (live) or nothing to fetch.
        let metadata = done.ok_or(ProviderError::Unavailable)?;
        if metadata
            .id
            .as_deref()
            .is_some_and(|id| id != request.id.as_str())
        {
            return Err(ProviderError::MalformedOutput);
        }
        let (media, thumbnail) =
            staged_files(request.staging, request.id, metadata.ext.as_deref())?;
        Ok(Outcome {
            media,
            thumbnail,
            metadata,
            merged: request.merger.is_some() && parts > 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_merges_up_to_the_ceiling_only_with_an_explicit_merger() {
        let reviewed = crate::test_absolute("/reviewed/ffmpeg");
        let merged = selection(2160, Some(&reviewed)).unwrap();
        let format = &merged[merged.iter().position(|a| a == "--format").unwrap() + 1];
        assert!(format.starts_with("bestvideo[height<=2160][protocol=https]+bestaudio"));
        // A progressive file under the ceiling is the fallback, never a larger one.
        assert!(
            format.ends_with("/best[height<=2160][protocol=https][vcodec!=none][acodec!=none]")
        );
        assert!(
            merged
                .windows(2)
                .any(|w| w[0] == "--ffmpeg-location" && Path::new(&w[1]) == reviewed)
        );
        assert!(
            merged
                .windows(2)
                .any(|w| w == ["--merge-output-format", "mp4/mkv"])
        );
        assert!(!merged.contains(&"--fixup".to_owned()));

        let single = selection(720, None).unwrap();
        let format = &single[single.iter().position(|a| a == "--format").unwrap() + 1];
        assert_eq!(
            format,
            "best[height<=720][protocol=https][vcodec!=none][acodec!=none]"
        );
        assert!(!format.contains('+'));
        assert!(!single.iter().any(|a| a.contains("ffmpeg")));
        assert!(single.windows(2).any(|w| w == ["--fixup", "never"]));

        assert!(selection(2161, None).is_err());
        assert!(selection(100, None).is_err());
        assert!(selection(1080, Some(Path::new("ffmpeg"))).is_err());
    }

    #[test]
    fn marker_lines_parse_missing_values_and_reject_everything_else() {
        assert_eq!(
            parse_line("OXPPLAN 137+140 1200"),
            Line::Plan {
                parts: 2,
                total: Some(1200)
            }
        );
        assert_eq!(
            parse_line("OXPPLAN 18 NA"),
            Line::Plan {
                parts: 1,
                total: None
            }
        );
        assert_eq!(
            parse_line("OXPPROG downloading 523264 5081689 NA 1611068.7 2 137"),
            Line::Progress(RawProgress {
                finished: false,
                downloaded: 523264,
                total: Some(5081689),
                estimate: None,
                speed: Some(1611068.7),
                eta: Some(2),
                part: "137".into(),
            })
        );
        let Line::Progress(finished) =
            parse_line("OXPPROG finished 5081689 NA 5000000.0 NA NA 251-drc")
        else {
            panic!("finished line")
        };
        assert!(finished.finished);
        assert_eq!(finished.estimate, Some(5_000_000));
        assert_eq!(finished.speed, None);
        assert_eq!(finished.part, "251-drc");
        for other in [
            "",
            "[download] 10% of 5MiB",
            "OXPPROG error 1 2 3 4 5 137",
            "OXPPROG downloading NA NA NA NA NA 137",
            "OXPPROG downloading 1 2",
            "OXPDONE not json",
            "OXPDONE [1,2]",
        ] {
            assert_eq!(parse_line(other), Line::Other, "{other:?}");
        }
        assert_eq!(
            parse_line(
                r#"OXPDONE {"id": "abcdefghijk", "title": "A \"quoted\"\u0007 title", "uploader": "Up", "duration": 12.5, "height": 720, "ext": "mp4"}"#
            ),
            Line::Done(Metadata {
                id: Some("abcdefghijk".into()),
                title: Some("A \"quoted\" title".into()),
                channel: Some("Up".into()),
                duration: Some(Duration::from_secs_f64(12.5)),
                height: Some(720),
                ext: Some("mp4".into()),
            })
        );
        // Fields the helper omits stay unknown.
        assert_eq!(
            parse_line(r#"OXPDONE {"id": "clip", "ext": "mp4"}"#),
            Line::Done(Metadata {
                id: Some("clip".into()),
                ext: Some("mp4".into()),
                ..Metadata::default()
            })
        );
    }

    fn feed(tracker: &mut Tracker, line: &str) -> Option<Progress> {
        tracker.apply(&parse_line(line))
    }

    #[test]
    fn merged_progress_is_weighted_by_the_planned_size_across_streams() {
        let mut tracker = Tracker::default();
        assert_eq!(feed(&mut tracker, "OXPPLAN 137+140 1000"), None);
        let first = feed(&mut tracker, "OXPPROG downloading 450 900 NA 100.0 5 137").unwrap();
        assert_eq!(first.fraction, Some(0.45));
        assert_eq!(first.total, Some(1000));
        // Whole-download estimate from the plan, not the video stream's ETA.
        assert_eq!(first.eta, Some(6));
        feed(&mut tracker, "OXPPROG finished 900 900 NA NA NA 137").unwrap();
        let audio = feed(&mut tracker, "OXPPROG downloading 50 100 NA 10.0 5 140").unwrap();
        assert_eq!(audio.fraction, Some(0.95));
        assert_eq!(audio.downloaded, 950);
        assert_eq!(audio.stage, Stage::Downloading);
        let done = feed(&mut tracker, "OXPPROG finished 100 100 NA NA NA 140").unwrap();
        assert_eq!(done.stage, Stage::Finishing);
        assert_eq!(done.fraction, Some(1.0));
        assert_eq!(done.eta, None);
    }

    #[test]
    fn unsized_streams_count_equally_and_a_single_file_uses_its_own_size() {
        let mut tracker = Tracker::default();
        feed(&mut tracker, "OXPPLAN 137+140 NA");
        let video = feed(&mut tracker, "OXPPROG downloading 50 100 NA 1.0 50 137").unwrap();
        assert_eq!(video.fraction, Some(0.25));
        assert_eq!(video.total, None);
        // Only the last stream's own ETA describes the remaining download.
        assert_eq!(video.eta, None);
        // A stream switch without a finished line still completes the first.
        let audio = feed(&mut tracker, "OXPPROG downloading 10 20 NA 1.0 10 140").unwrap();
        assert_eq!(audio.fraction, Some(0.75));
        assert_eq!(audio.eta, Some(10));

        let mut single = Tracker::default();
        feed(&mut single, "OXPPLAN 18 NA");
        let progress = feed(&mut single, "OXPPROG downloading 25 NA 100 4.0 19 18").unwrap();
        assert_eq!(progress.fraction, Some(0.25));
        assert_eq!(progress.total, Some(100));
        assert_eq!(progress.eta, Some(19));
        // An underestimated size never shows more than 99 % before the end.
        let over = feed(&mut single, "OXPPROG downloading 150 NA 100 4.0 0 18").unwrap();
        assert_eq!(over.fraction, Some(0.99));
        assert_eq!(over.total, Some(150));
    }

    #[cfg(unix)]
    mod helper {
        use super::*;
        use oxplay_core::CancellationToken;
        use std::os::unix::fs::PermissionsExt;

        pub(super) struct Fixture {
            pub root: PathBuf,
            pub helper: PathBuf,
            pub staging: PathBuf,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
        /// A synthetic helper script: it never contacts the network. `body`
        /// runs with `$dir` set to the value following `--paths`.
        pub(super) fn fixture(body: &str) -> Fixture {
            let root = std::env::temp_dir().join(format!(
                "oxplay-download-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let staging = root.join("staging");
            std::fs::create_dir_all(&staging).unwrap();
            let helper = root.join("yt-dlp");
            std::fs::write(
                &helper,
                format!(
                    "#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = --paths ]; then dir=\"$2\"; fi; shift; done\n{body}\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
            Fixture {
                root,
                helper,
                staging,
            }
        }
        pub(super) fn op() -> OperationContext {
            OperationContext {
                request_id: 1,
                session_generation: 0,
                cancel: CancellationToken::default(),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn synthetic_helper_download_reports_progress_and_staged_files() {
        let fixture = helper::fixture(
            r#"echo "OXPPLAN 18 400"
echo "OXPPROG downloading 100 400 NA 50.0 6 18"
echo "OXPPROG finished 400 400 NA NA NA 18"
printf 'media' > "$dir/abcdefghijk.mp4"
printf 'art' > "$dir/abcdefghijk.webp"
echo 'OXPDONE {"id": "abcdefghijk", "title": "Fixture", "channel": "Maker", "height": 360, "ext": "mp4"}'"#,
        );
        let provider = YtDlp::new(&fixture.helper).unwrap();
        let id = VideoId::new("abcdefghijk").unwrap();
        let mut seen = Vec::new();
        let outcome = provider
            .download(
                &Request {
                    id: &id,
                    max_height: 1080,
                    merger: None,
                    staging: &fixture.staging,
                },
                &helper::op(),
                &mut |progress| seen.push(progress),
            )
            .unwrap();
        assert_eq!(outcome.media, fixture.staging.join("abcdefghijk.mp4"));
        assert_eq!(
            outcome.thumbnail,
            Some(fixture.staging.join("abcdefghijk.webp"))
        );
        assert_eq!(outcome.metadata.title.as_deref(), Some("Fixture"));
        assert_eq!(outcome.metadata.height, Some(360));
        assert!(!outcome.merged);
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].fraction, Some(0.25));
        assert_eq!(seen[1].stage, Stage::Finishing);
    }

    #[cfg(unix)]
    #[test]
    fn synthetic_helper_failures_are_classified_and_rate_limits_start_a_cooldown() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let fixture =
            helper::fixture("echo 'ERROR: HTTP Error 429: Too Many Requests' >&2; exit 1");
        let provider = YtDlp::new(&fixture.helper).unwrap();
        let request = Request {
            id: &id,
            max_height: 720,
            merger: None,
            staging: &fixture.staging,
        };
        assert_eq!(
            provider
                .download(&request, &helper::op(), &mut |_| {})
                .unwrap_err(),
            ProviderError::RateLimited
        );
        // The shared cooldown now refuses without spawning another helper.
        assert_eq!(
            provider
                .download(&request, &helper::op(), &mut |_| {})
                .unwrap_err(),
            ProviderError::RateLimited
        );
        // Exit 0 with nothing downloaded (e.g. a live stream filtered out).
        let fixture = helper::fixture("exit 0");
        let provider = YtDlp::new(&fixture.helper).unwrap();
        let request = Request {
            staging: &fixture.staging,
            ..request
        };
        assert_eq!(
            provider
                .download(&request, &helper::op(), &mut |_| {})
                .unwrap_err(),
            ProviderError::Unavailable
        );
        // A reported file that is missing, or a different video, is rejected.
        let fixture = helper::fixture(r#"echo 'OXPDONE {"id": "abcdefghijk", "ext": "mp4"}'"#);
        let provider = YtDlp::new(&fixture.helper).unwrap();
        let request = Request {
            staging: &fixture.staging,
            ..request
        };
        assert_eq!(
            provider
                .download(&request, &helper::op(), &mut |_| {})
                .unwrap_err(),
            ProviderError::MalformedOutput
        );
        let fixture = helper::fixture(
            r#"printf x > "$dir/abcdefghijk.mp4"; echo 'OXPDONE {"id": "zzzzzzzzzzz", "ext": "mp4"}'"#,
        );
        let provider = YtDlp::new(&fixture.helper).unwrap();
        let request = Request {
            staging: &fixture.staging,
            ..request
        };
        assert_eq!(
            provider
                .download(&request, &helper::op(), &mut |_| {})
                .unwrap_err(),
            ProviderError::MalformedOutput
        );
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_a_synthetic_download_stops_the_helper_promptly() {
        let fixture =
            helper::fixture("echo 'OXPPROG downloading 1 100 NA 1.0 99 18'; sleep 30 & wait");
        let provider = YtDlp::new(&fixture.helper).unwrap();
        let id = VideoId::new("abcdefghijk").unwrap();
        let operation = helper::op();
        let cancel = operation.cancel.clone();
        let started = std::time::Instant::now();
        let result = provider.download(
            &Request {
                id: &id,
                max_height: 480,
                merger: None,
                staging: &fixture.staging,
            },
            &operation,
            &mut |_| cancel.cancel(),
        );
        assert_eq!(result.unwrap_err(), ProviderError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn staged_files_accept_only_regular_id_named_outputs() {
        let root = std::env::temp_dir().join(format!(
            "oxplay-staged-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let id = VideoId::new("abcdefghijk").unwrap();
        assert!(staged_files(&root, &id, Some("mp4")).is_err());
        std::fs::write(root.join("abcdefghijk.mkv"), b"media").unwrap();
        std::fs::write(root.join("other.mp4"), b"media").unwrap();
        std::fs::write(root.join("abcdefghijk.exe"), b"no").unwrap();
        // A wrong or unsupported reported extension falls back to a real output.
        let (media, thumbnail) = staged_files(&root, &id, Some("exe")).unwrap();
        assert_eq!(media, root.join("abcdefghijk.mkv"));
        assert_eq!(thumbnail, None);
        std::fs::write(root.join("abcdefghijk.jpg"), b"").unwrap();
        assert_eq!(staged_files(&root, &id, None).unwrap().1, None);
        std::fs::write(root.join("abcdefghijk.jpg"), b"art").unwrap();
        assert_eq!(
            staged_files(&root, &id, Some("mkv")).unwrap().1,
            Some(root.join("abcdefghijk.jpg"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
