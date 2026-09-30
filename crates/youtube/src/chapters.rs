// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional extractor chapters: discard an invalid set, never invent ranges.
use serde_json::Value;
use serein_core::{MAX_VIDEO_CHAPTERS, ProviderError, VideoChapter};
use std::time::Duration;

fn seconds(value: &Value) -> Result<Duration, ProviderError> {
    let seconds = value.as_f64().ok_or(ProviderError::MalformedOutput)?;
    if !seconds.is_finite() || seconds < 0. || seconds > f64::from(u32::MAX) {
        return Err(ProviderError::MalformedOutput);
    }
    Duration::try_from_secs_f64(seconds).map_err(|_| ProviderError::MalformedOutput)
}
pub(crate) fn parse(value: &Value) -> Result<Vec<VideoChapter>, ProviderError> {
    let Some(raw) = value.get("chapters").filter(|raw| !raw.is_null()) else {
        return Ok(Vec::new());
    };
    let list = raw.as_array().ok_or(ProviderError::MalformedOutput)?;
    if list.len() > MAX_VIDEO_CHAPTERS {
        return Err(ProviderError::OutputTooLarge);
    }
    let duration = value
        .get("duration")
        .filter(|raw| !raw.is_null())
        .map(seconds)
        .transpose()?;
    let mut chapters = Vec::with_capacity(list.len());
    let mut previous_end = Duration::ZERO;
    for entry in list {
        let start = seconds(&entry["start_time"])?;
        let end = seconds(&entry["end_time"])?;
        if start >= end || start < previous_end || duration.is_some_and(|duration| end > duration) {
            return Err(ProviderError::MalformedOutput);
        }
        let title = match entry.get("title").filter(|title| !title.is_null()) {
            None => None,
            Some(title) => chapter_title(title.as_str().ok_or(ProviderError::MalformedOutput)?)?,
        };
        chapters.push(VideoChapter { title, start, end });
        previous_end = end;
    }
    Ok(chapters)
}
fn chapter_title(title: &str) -> Result<Option<String>, ProviderError> {
    if title.len() > 1024 || title.chars().count() > 256 || title.chars().any(char::is_control) {
        return Err(ProviderError::MalformedOutput);
    }
    Ok((!title.trim().is_empty()).then(|| title.trim().to_owned()))
}
/// Native watch-page markers carry only start offsets. Each range ends at the
/// next start and the final range at the resolved duration; without a duration
/// the set cannot be closed and is not published. The same bounds apply.
pub(crate) fn from_starts(
    starts: &[(Option<String>, Duration)],
    duration: Option<Duration>,
) -> Result<Vec<VideoChapter>, ProviderError> {
    if starts.len() > MAX_VIDEO_CHAPTERS {
        return Err(ProviderError::OutputTooLarge);
    }
    let duration = duration.ok_or(ProviderError::Unavailable)?;
    let mut chapters = Vec::with_capacity(starts.len());
    for (index, (title, start)) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(duration, |(_, next)| *next);
        if *start >= end || end > duration {
            return Err(ProviderError::MalformedOutput);
        }
        let title = match title {
            Some(title) => chapter_title(title)?,
            None => None,
        };
        chapters.push(VideoChapter {
            title,
            start: *start,
            end,
        });
    }
    Ok(chapters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn genuine_ordered_ranges_keep_subseconds_and_optional_titles() {
        let parsed = parse(&json!({"duration": 90.5, "chapters": [
            {"start_time": 0, "end_time": 12.5, "title": "First"},
            {"start_time": 12.5, "end_time": 90.5}
        ]}))
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].start, Duration::from_millis(12_500));
        assert_eq!(parsed[1].title, None);
        assert!(parse(&json!({})).unwrap().is_empty());
    }
    #[test]
    fn malformed_or_oversized_sets_never_publish_partial_chapter_navigation() {
        for chapters in [
            json!([{ "start_time": -1, "end_time": 10 }]),
            json!([{ "start_time": 10, "end_time": 10 }]),
            json!([{ "start_time": 0, "end_time": 100 }]),
            json!([{ "start_time": 0, "end_time": 10 }, { "start_time": 9, "end_time": 20 }]),
            json!([{ "start_time": 0, "end_time": "NaN" }]),
            json!([{ "start_time": 0, "end_time": 10, "title": "x".repeat(1025) }]),
            json!(vec![
                json!({"start_time":0,"end_time":1});
                MAX_VIDEO_CHAPTERS + 1
            ]),
        ] {
            assert!(parse(&json!({"duration":90,"chapters":chapters})).is_err());
        }
        assert!(seconds(&Value::Null).is_err());
        assert!(seconds(&json!(f64::MAX)).is_err());
    }
    #[test]
    fn start_only_markers_close_at_the_next_start_and_resolved_duration() {
        let s = Duration::from_secs;
        let starts = [(Some("Intro".to_owned()), s(0)), (None, s(30))];
        let chapters = from_starts(&starts, Some(s(90))).unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].end, s(30));
        assert_eq!(chapters[1].end, s(90));
        assert_eq!(chapters[1].title, None);
        assert!(
            from_starts(&starts, None).is_err(),
            "no duration, no ranges"
        );
        assert!(
            from_starts(&starts, Some(s(30))).is_err(),
            "start beyond end"
        );
        let unordered = [(None, s(10)), (None, s(10))];
        assert!(from_starts(&unordered, Some(s(90))).is_err());
        let control = [(Some("a\u{0007}".to_owned()), s(0))];
        assert!(from_starts(&control, Some(s(90))).is_err());
        assert!(from_starts(&[], Some(s(90))).unwrap().is_empty());
    }
}
