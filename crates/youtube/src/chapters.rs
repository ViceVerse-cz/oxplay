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
            Some(title) => {
                let title = title.as_str().ok_or(ProviderError::MalformedOutput)?;
                if title.len() > 1024
                    || title.chars().count() > 256
                    || title.chars().any(char::is_control)
                {
                    return Err(ProviderError::MalformedOutput);
                }
                (!title.trim().is_empty()).then(|| title.trim().to_owned())
            }
        };
        chapters.push(VideoChapter { title, start, end });
        previous_end = end;
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
}
