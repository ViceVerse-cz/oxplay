// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed per-file command options avoid parsing signed URLs as comma-separated
//! command text. ABI/source contract: mpv 0.41 client.h and command.c:cmd_loadfile.
use crate::{MediaError, Result, checked, cstring, ffi};
use std::ffi::{CStr, CString};

fn string(value: &CStr) -> ffi::Node {
    ffi::Node {
        data: ffi::NodeData {
            string: value.as_ptr().cast_mut(),
        },
        format: 1,
    }
}
fn list(value: &mut ffi::NodeList, map: bool) -> ffi::Node {
    ffi::Node {
        data: ffi::NodeData {
            list: std::ptr::from_mut(value),
        },
        format: if map { 8 } else { 7 },
    }
}

/// All input is borrowed only through this call. client.h's MPV_FORMAT_NODE
/// writing contract says the API never writes to this storage and copies as
/// needed. mpv_command_node_async parses/copies before returning; no Rust pointer
/// is retained by queued execution. Options belong to the exact playlist entry.
///
/// # Safety
/// `raw` must be the live owned player handle and the caller must enforce its
/// command-queue admission bound. No call is made from an mpv callback.
pub(super) unsafe fn loadfile(
    raw: *mut ffi::Handle,
    reply: u64,
    path: &str,
    start: &str,
    audio: Option<&str>,
    subtitle: Option<&str>,
) -> Result<()> {
    if [Some(path), Some(start), audio, subtitle]
        .into_iter()
        .flatten()
        .any(|value| value.len() > 65_536)
    {
        return Err(MediaError("Media argument exceeds the 64 KiB limit".into()));
    }
    let args = [
        cstring("loadfile")?,
        cstring(path)?,
        cstring("replace")?,
        cstring("-1")?,
    ];
    // -append deliberately treats the complete input
    // as one filename (m_option.c:separate_input_param, OP_APPEND separator=0).
    let mut keys = vec![c"start"];
    let mut value_strings = vec![cstring(start)?];
    if std::path::Path::new(path).is_absolute() {
        // Local inputs must stay within file containers, including when a
        // selected file is replaced after the host's header validation. Force
        // lavf so mpv's playlist/EDL/image-sequence demuxers cannot take over.
        // access-references=no additionally rejects lavf nested io_open calls.
        // Subtitle files inherit this entry's options: allow their formats but
        // never force the primary video's specific format onto a subtitle.
        // mpv 0.41 m_option.c:read_subparam accepts [quoted,comma,values].
        for (key, value) in [
            (c"demuxer", "lavf"),
            (c"sub-demuxer", "lavf"),
            (
                c"demuxer-lavf-o",
                "format_whitelist=[mov,matroska,webm,avi,wav,flac,mp3,ogg,srt,ass,webvtt]",
            ),
        ] {
            keys.push(key);
            value_strings.push(cstring(value)?);
        }
    }
    if let Some(audio) = audio {
        keys.push(c"audio-files-append");
        value_strings.push(cstring(audio)?);
    }
    if let Some(subtitle) = subtitle {
        keys.push(c"sub-files-append");
        value_strings.push(cstring(subtitle)?);
    }
    let mut keys: Vec<_> = keys.iter().map(|key| key.as_ptr().cast_mut()).collect();
    let mut values: Vec<_> = value_strings
        .iter()
        .map(CString::as_c_str)
        .map(string)
        .collect();
    let mut options = ffi::NodeList {
        num: values.len() as i32,
        values: values.as_mut_ptr(),
        keys: keys.as_mut_ptr(),
    };
    let mut nodes: Vec<_> = args.iter().map(CString::as_c_str).map(string).collect();
    nodes.push(list(&mut options, true));
    let mut arguments = ffi::NodeList {
        num: nodes.len() as i32,
        values: nodes.as_mut_ptr(),
        keys: std::ptr::null_mut(),
    };
    let mut command = list(&mut arguments, false);
    unsafe {
        checked(
            ffi::mpv_command_node_async(raw, reply, &mut command),
            "Load media",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::c_void,
        path::Path,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    unsafe extern "C" {
        fn mpv_get_property(
            raw: *mut ffi::Handle,
            name: *const std::ffi::c_char,
            format: i32,
            value: *mut c_void,
        ) -> i32;
    }
    struct Engine(*mut ffi::Handle);
    impl Drop for Engine {
        fn drop(&mut self) {
            unsafe {
                ffi::mpv_terminate_destroy(self.0);
            }
        }
    }
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn wav(path: &Path) {
        // Half a second of silence; the test explicitly selects null audio/VO.
        let samples = 24_000u32;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + samples * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48_000u32.to_le_bytes());
        bytes.extend(96_000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((samples * 2).to_le_bytes());
        bytes.resize(44 + samples as usize * 2, 0);
        std::fs::write(path, bytes).unwrap();
    }
    fn track_count(engine: &Engine) -> Option<i64> {
        let mut result = 0i64;
        let code = unsafe {
            mpv_get_property(
                engine.0,
                c"track-list/count".as_ptr(),
                4,
                std::ptr::from_mut(&mut result).cast(),
            )
        };
        (code >= 0).then_some(result)
    }
    fn await_tracks(engine: &Engine, expected: i64) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let event = unsafe { &*ffi::mpv_wait_event(engine.0, 0.05) };
            if event.id == 5 {
                assert!(event.error >= 0, "native command error {}", event.error);
            }
            if event.id == 8 && track_count(engine) == Some(expected) {
                return;
            }
        }
        panic!(
            "expected {expected} native tracks, observed {:?}",
            track_count(engine)
        );
    }
    #[test]
    fn file_options_keep_punctuated_companion_paths_scoped_across_replacement() {
        let fixture = Fixture(std::env::temp_dir().join(format!(
                "serein-node-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        std::fs::create_dir(&fixture.0).unwrap();
        let first = fixture.0.join("first.wav");
        let second = fixture.0.join("second.wav");
        let audio = fixture.0.join("audio,extra=日本.wav");
        let subtitle = fixture.0.join("caption,extra=日本.srt");
        for path in [&first, &second, &audio] {
            wav(path);
        }
        std::fs::write(
            &subtitle,
            "1\n00:00:00,000 --> 00:00:00,400\nScoped fixture\n",
        )
        .unwrap();
        let engine = Engine(unsafe { ffi::mpv_create() });
        assert!(!engine.0.is_null());
        for (key, value) in [
            (c"config", c"no"),
            (c"terminal", c"no"),
            (c"load-scripts", c"no"),
            (c"ytdl", c"no"),
            (c"vo", c"null"),
            (c"ao", c"null"),
            (c"pause", c"yes"),
            (c"idle", c"yes"),
            (c"audio-file-auto", c"no"),
            (c"sub-auto", c"no"),
        ] {
            assert!(
                unsafe { ffi::mpv_set_option_string(engine.0, key.as_ptr(), value.as_ptr()) } >= 0
            );
        }
        assert!(unsafe { ffi::mpv_initialize(engine.0) } >= 0);
        unsafe {
            loadfile(
                engine.0,
                1,
                first.to_str().unwrap(),
                "0.05",
                audio.to_str(),
                subtitle.to_str(),
            )
            .unwrap();
        }
        await_tracks(&engine, 3);
        // No FILE_LOADED handler attaches resources: replacement restores options.
        unsafe {
            loadfile(
                engine.0,
                2,
                first.to_str().unwrap(),
                "0",
                audio.to_str(),
                subtitle.to_str(),
            )
            .unwrap();
            loadfile(engine.0, 3, second.to_str().unwrap(), "0", None, None).unwrap();
        }
        await_tracks(&engine, 1);
    }
}
