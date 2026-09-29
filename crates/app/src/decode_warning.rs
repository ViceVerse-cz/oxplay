// SPDX-License-Identifier: GPL-3.0-or-later
//! Presentation labels for observed mpv 0.41 decoder methods. These are not an
//! end-to-end transfer classification or a performance acceptance result.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    Inactive,
    Software,
    Hardware,
    CopyingHardware,
    Unknown,
}

fn classify(active_video: bool, method: &str) -> Path {
    if !active_video {
        return Path::Inactive;
    }
    // Pinned mpv v0.41.0 video/decode/vd_lavc.c hwdec_autoprobe_info.
    // add_hwdec_item appends -copy when copying=true; decoded output either
    // receives software frames or downloads remaining hardware frames.
    match method {
        "no" => Path::Software,
        "d3d11va-copy" | "vulkan-copy" | "dxva2-copy" | "nvdec-copy" | "vaapi-copy"
        | "vdpau-copy" | "drm-copy" | "mediacodec-copy" | "videotoolbox-copy" => {
            Path::CopyingHardware
        }
        "d3d11va" | "vulkan" | "dxva2" | "nvdec" | "vaapi" | "vdpau" | "drm" | "mediacodec"
        | "videotoolbox" => Path::Hardware,
        // New/unreviewed method names require qualification, even if their
        // spelling resembles a known method. Never infer zero-copy here.
        _ => Path::Unknown,
    }
}

pub fn warning(silent_audio: bool, active_video: bool, method: &str) -> &'static str {
    if silent_audio {
        return "DIAGNOSTIC: audio output is silent. This run cannot pass playback acceptance.";
    }
    match classify(active_video, method) {
        Path::Inactive | Path::Hardware => "",
        Path::Software => {
            "Software video decoding is active. This is not the hardware playback path."
        }
        Path::CopyingHardware => {
            "Hardware decoding is using CPU-accessible frames. This fallback is not the optimized playback path."
        }
        Path::Unknown => {
            "The active video decoder is unverified. Hardware playback has not been confirmed."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reviewed_copy_methods_are_fallbacks_without_relabeling_direct_hardware() {
        for direct in [
            "d3d11va",
            "vulkan",
            "dxva2",
            "nvdec",
            "vaapi",
            "vdpau",
            "drm",
            "mediacodec",
            "videotoolbox",
        ] {
            assert_eq!(classify(true, direct), Path::Hardware);
            assert_eq!(warning(false, true, direct), "");
            assert_eq!(
                classify(true, &format!("{direct}-copy")),
                Path::CopyingHardware
            );
            assert!(!warning(false, true, &format!("{direct}-copy")).is_empty());
        }
        assert_eq!(classify(true, "no"), Path::Software);
        for unknown in [
            "",
            "auto-safe",
            "unverified",
            "future-copy",
            "videotoolbox-copy-extra",
            "VIDEOTOOLBOX",
        ] {
            assert_eq!(classify(true, unknown), Path::Unknown);
            assert!(!warning(false, true, unknown).is_empty());
        }
    }
    #[test]
    fn audio_only_and_silent_diagnostic_have_explicit_priority() {
        for method in ["no", "videotoolbox-copy", "videotoolbox", ""] {
            assert_eq!(warning(false, false, method), "");
            assert!(warning(true, false, method).starts_with("DIAGNOSTIC: audio output is silent"));
            assert_eq!(warning(true, false, method), warning(true, true, method));
        }
    }
}
