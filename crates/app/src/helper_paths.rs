//! Resolve explicit helpers or the owning application bundle. Never search PATH,
//! except the Windows development fallback documented in `windows` below.
use std::path::{Path, PathBuf};

pub struct HelperPaths {
    pub yt_dlp: PathBuf,
    pub deno: PathBuf,
    pub media_ca: Option<PathBuf>,
    pub dns_helper: Option<PathBuf>,
    bundle_resources: Option<PathBuf>,
}

impl HelperPaths {
    pub fn discover(
        executable: &Path,
        yt_dlp: Option<PathBuf>,
        deno: Option<PathBuf>,
    ) -> Result<Self, &'static str> {
        let bundled = executable.parent().and_then(|macos| {
            let contents = macos.parent()?;
            let application = contents.parent()?;
            (macos.file_name()? == "MacOS"
                && contents.file_name()? == "Contents"
                && application
                    .extension()?
                    .to_str()?
                    .eq_ignore_ascii_case("app"))
            .then(|| contents.join("Helpers"))
        });
        let bundle_resources = bundled
            .as_ref()
            .and_then(|directory| directory.parent())
            .map(|contents| contents.join("Resources"));
        let dns_helper = if cfg!(target_os = "macos") {
            Some(
                bundled
                    .as_ref()
                    .map(|directory| directory.join("oxplay-dns"))
                    .or_else(|| {
                        executable
                            .parent()
                            .map(|directory| directory.join("oxplay-dns"))
                    })
                    .ok_or("The native DNS helper location is unavailable")?,
            )
        } else {
            None
        };
        let media_ca = bundle_resources
            .as_ref()
            .map(|resources| resources.join("Certificates/mozilla.pem"))
            .or_else(|| {
                cfg!(target_os = "macos").then(|| {
                    PathBuf::from(
                        "/opt/homebrew/opt/ca-certificates/share/ca-certificates/cacert.pem",
                    )
                })
            });
        #[cfg(windows)]
        let (yt_dlp, deno) = (
            yt_dlp.or_else(|| windows::helper(executable, "yt-dlp.exe")),
            deno.or_else(|| windows::helper(executable, "deno.exe")),
        );
        let directory = bundled.unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                PathBuf::from("/opt/homebrew/bin")
            } else {
                PathBuf::from("/usr/bin")
            }
        });
        // Keep missing bundled paths: the worker reports unavailable instead of
        // silently executing an unrelated system/Homebrew helper.
        let paths = Self {
            yt_dlp: yt_dlp.unwrap_or_else(|| directory.join("yt-dlp")),
            deno: deno.unwrap_or_else(|| directory.join("deno")),
            media_ca,
            dns_helper,
            bundle_resources,
        };
        if !paths.yt_dlp.is_absolute() || !paths.deno.is_absolute() {
            return Err("Helper paths must be absolute");
        }
        Ok(paths)
    }

    /// Only needed for the opt-in scoped media transport. Check before the UI
    /// starts; default browsing/direct playback does not launch or require it.
    pub fn validate_dns_helper(&self) -> Result<(), &'static str> {
        let Some(path) = &self.dns_helper else {
            return Ok(());
        };
        if !path.is_absolute() {
            return Err("The native DNS helper path must be absolute");
        }
        let metadata = std::fs::metadata(path)
            .map_err(|_| "The native DNS helper is unavailable; build or install oxplay-dns")?;
        if !metadata.is_file() {
            return Err("The native DNS helper must be a regular executable file");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err("The native DNS helper is not executable");
            }
        }
        if let Some(resources) = &self.bundle_resources {
            let contents = resources
                .parent()
                .ok_or("Invalid application resource directory")?
                .canonicalize()
                .map_err(|_| "The application directory is unavailable")?;
            let helper = path
                .canonicalize()
                .map_err(|_| "The native DNS helper is unavailable")?;
            if !helper.starts_with(contents.join("Helpers")) {
                return Err("The native DNS helper must remain inside this application");
            }
        }
        Ok(())
    }

    /// Called before the UI event loop. A broken packaged trust resource never
    /// silently changes to a host/default CA store. TLS parses the certificates.
    pub fn validate_media_ca(&self) -> Result<(), &'static str> {
        let Some(path) = &self.media_ca else {
            return Ok(());
        };
        if !path.is_absolute() || path.to_str().is_none() {
            return Err("The media certificate bundle path must be absolute UTF-8");
        }
        let metadata = std::fs::metadata(path)
            .map_err(|_| "This build's media certificate bundle is missing or unreadable")?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 4 * 1024 * 1024 {
            return Err("This build's media certificate bundle has an invalid file type or size");
        }
        if let Some(resources) = &self.bundle_resources {
            let resources = resources
                .canonicalize()
                .map_err(|_| "The application resource directory is unavailable")?;
            let certificate = path
                .canonicalize()
                .map_err(|_| "The media certificate bundle is unavailable")?;
            if !certificate.starts_with(resources) {
                return Err("The media certificate bundle must remain inside this application");
            }
        }
        Ok(())
    }
}

/// Windows has no application bundle. A portable/installed Oxplay ships its
/// helpers beside `oxplay.exe`; that sibling always wins. Only when it is
/// absent (development checkouts) is one absolute `PATH` entry accepted, e.g.
/// a winget/scoop/pip `yt-dlp.exe`. Relative `PATH` entries are ignored so the
/// working directory can never supply a helper. If neither exists, the sibling
/// path is kept and the worker reports the helper as unavailable.
#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};

    pub(super) fn helper(executable: &Path, name: &str) -> Option<PathBuf> {
        let sibling = executable.parent()?.join(name);
        if sibling.is_file() {
            return Some(sibling);
        }
        search(std::env::var_os("PATH").as_deref(), name).or(Some(sibling))
    }

    pub(super) fn search(path: Option<&std::ffi::OsStr>, name: &str) -> Option<PathBuf> {
        std::env::split_paths(path?)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn sibling_helper_wins_and_path_search_ignores_relative_entries() {
            let root = std::env::temp_dir().join(format!(
                "oxplay-windows-helper-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let application = root.join("Oxplay");
            let tools = root.join("tools");
            std::fs::create_dir_all(&application).unwrap();
            std::fs::create_dir_all(&tools).unwrap();
            let executable = application.join("oxplay.exe");
            std::fs::write(tools.join("yt-dlp.exe"), b"not executed").unwrap();
            let path = std::env::join_paths([PathBuf::from("relative"), tools.clone()]).unwrap();
            assert_eq!(
                search(Some(&path), "yt-dlp.exe"),
                Some(tools.join("yt-dlp.exe"))
            );
            let relative_only = std::env::join_paths([PathBuf::from("tools")]).unwrap();
            assert_eq!(search(Some(&relative_only), "yt-dlp.exe"), None);
            assert_eq!(search(Some(&path), "deno.exe"), None);
            std::fs::write(application.join("yt-dlp.exe"), b"not executed").unwrap();
            assert_eq!(
                helper(&executable, "yt-dlp.exe"),
                Some(application.join("yt-dlp.exe"))
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unix-rooted synthetic paths are not absolute on Windows.
    #[cfg(unix)]
    #[test]
    fn missing_bundle_helpers_never_fall_back_to_host() {
        let paths = HelperPaths::discover(
            Path::new("/nonexistent/Oxplay.app/Contents/MacOS/oxplay"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            paths.yt_dlp,
            Path::new("/nonexistent/Oxplay.app/Contents/Helpers/yt-dlp")
        );
        assert_eq!(
            paths.deno,
            Path::new("/nonexistent/Oxplay.app/Contents/Helpers/deno")
        );
        assert_eq!(
            paths.media_ca.as_deref(),
            Some(Path::new(
                "/nonexistent/Oxplay.app/Contents/Resources/Certificates/mozilla.pem"
            ))
        );
        assert!(paths.validate_media_ca().is_err());
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                paths.dns_helper.as_deref(),
                Some(Path::new(
                    "/nonexistent/Oxplay.app/Contents/Helpers/oxplay-dns"
                ))
            );
            assert!(paths.validate_dns_helper().is_err());
        }
    }

    // Unix-rooted synthetic paths are not absolute on Windows.
    #[cfg(unix)]
    #[test]
    fn explicit_paths_override_only_the_selected_helper_and_must_be_absolute() {
        let executable = Path::new("/Oxplay.app/Contents/MacOS/oxplay");
        let paths =
            HelperPaths::discover(executable, Some("/reviewed/yt-dlp".into()), None).unwrap();
        assert_eq!(paths.yt_dlp, Path::new("/reviewed/yt-dlp"));
        assert_eq!(paths.deno, Path::new("/Oxplay.app/Contents/Helpers/deno"));
        assert!(HelperPaths::discover(executable, None, Some("deno".into())).is_err());
        let uppercase =
            HelperPaths::discover(Path::new("/Renamed.APP/Contents/MacOS/oxplay"), None, None)
                .unwrap();
        assert_eq!(
            uppercase.deno,
            Path::new("/Renamed.APP/Contents/Helpers/deno")
        );
    }

    #[test]
    fn packaged_ca_must_be_a_nonempty_regular_resource() {
        let temporary = std::env::temp_dir().join(format!(
            "oxplay-ca-resource-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = HelperPaths::discover(
            &temporary.join("Oxplay.app/Contents/MacOS/oxplay"),
            None,
            None,
        )
        .unwrap();
        let ca = paths.media_ca.as_ref().unwrap();
        std::fs::create_dir_all(ca.parent().unwrap()).unwrap();
        std::fs::write(ca, b"").unwrap();
        assert!(paths.validate_media_ca().is_err());
        std::fs::write(ca, b"synthetic path fixture; never used for TLS").unwrap();
        assert!(paths.validate_media_ca().is_ok());
        std::fs::remove_file(ca).unwrap();
        std::fs::create_dir(ca).unwrap();
        assert!(paths.validate_media_ca().is_err());
        std::fs::remove_dir_all(temporary).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_dns_helper_uses_only_sibling_or_bundle_and_rejects_escape() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let paths = HelperPaths::discover(Path::new("/reviewed/target/release/oxplay"), None, None)
            .unwrap();
        assert_eq!(
            paths.dns_helper.as_deref(),
            Some(Path::new("/reviewed/target/release/oxplay-dns"))
        );
        let temporary = std::env::temp_dir().join(format!(
            "oxplay-dns-path-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = HelperPaths::discover(
            &temporary.join("Oxplay.app/Contents/MacOS/oxplay"),
            None,
            None,
        )
        .unwrap();
        let helper = paths.dns_helper.as_ref().unwrap();
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(helper, b"not executed; path validation fixture").unwrap();
        std::fs::set_permissions(helper, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(paths.validate_dns_helper().is_err());
        std::fs::set_permissions(helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(paths.validate_dns_helper().is_ok());
        std::fs::remove_file(helper).unwrap();
        let outside = temporary.join("outside-helper");
        std::fs::write(&outside, b"not executed").unwrap();
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&outside, helper).unwrap();
        assert!(paths.validate_dns_helper().is_err());
        std::fs::remove_dir_all(temporary).unwrap();
    }
}
