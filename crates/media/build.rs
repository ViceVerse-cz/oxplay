use std::path::{Path, PathBuf};

/// libmpv client API 2.5 (mpv 0.41) is the minimum: frame timing depends on it.
const MIN_CLIENT_API: (u32, u32) = (2, 5);

fn main() {
    println!("cargo:rerun-if-changed=src/native_child.m");
    println!("cargo:rerun-if-env-changed=MPV_DIR");
    println!("cargo:rerun-if-env-changed=OXPLAY_NATIVE_MPV_PREFIX");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        cc::Build::new()
            .file("src/native_child.m")
            .flag("-fobjc-arc")
            .flag("-Wno-deprecated-declarations")
            .compile("oxplay_native_child");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=OpenGL");
    }
    if std::env::var_os("CARGO_FEATURE_NATIVE_RENDERING").is_some() {
        let prefix = std::env::var_os("OXPLAY_NATIVE_MPV_PREFIX")
            .or_else(|| {
                (target_os == "windows")
                    .then(|| std::env::var_os("MPV_DIR"))
                    .flatten()
            })
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
                    .join("../../artifacts/native-media")
                    .join(&target_os)
                    .join("prefix")
            });
        link_native_mpv(&prefix, &target_os);
        return;
    }
    if target_os == "windows"
        && let Some(directory) = std::env::var_os("MPV_DIR").filter(|value| !value.is_empty())
    {
        link_windows_mpv_dir(Path::new(&directory));
        return;
    }
    pkg_config::Config::new()
        .atleast_version("2.5")
        .probe("mpv")
        .expect(
            "libmpv development files required (macOS: brew install mpv; Linux: libmpv-dev; \
             Windows: set MPV_DIR to an extracted mpv-dev archive, see README)",
        );
}

/// Native rendering uses an explicitly versioned, private media ABI. A stock
/// libmpv exposes the same client symbols, so a successful link is insufficient.
fn link_native_mpv(prefix: &Path, target_os: &str) {
    let prefix = prefix.canonicalize().unwrap_or_else(|_| {
        panic!(
            "Native media prefix {} is missing. Build scripts/native-media/{target_os}.py first, \
         or set OXPLAY_NATIVE_MPV_PREFIX to its installed prefix. For the GL comparison \
         use --no-default-features.",
            prefix.display()
        )
    });
    let api = match target_os {
        "macos" => "mtl",
        "linux" => "vk",
        "windows" => "d3d11",
        _ => panic!("Native media is unsupported on {target_os}"),
    };
    let header = prefix.join(format!("include/mpv/render_{api}.h"));
    let text = std::fs::read_to_string(&header)
        .unwrap_or_else(|_| panic!("Native media prefix is missing {}", header.display()));
    assert!(
        text.lines()
            .any(|line| line.trim() == "#define OXPLAY_NATIVE_RENDER_ABI 1"),
        "{} must provide OXPLAY_NATIVE_RENDER_ABI 1; stock/other experimental libmpv is incompatible",
        header.display()
    );
    println!("cargo:rerun-if-changed={}", header.display());
    if target_os == "windows" {
        link_windows_mpv_dir(&prefix);
    } else {
        let lib = prefix.join("lib");
        let filename = if target_os == "macos" {
            "libmpv.dylib"
        } else {
            "libmpv.so"
        };
        assert!(
            lib.join(filename).is_file(),
            "Native prefix is missing {filename}"
        );
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=dylib=mpv");
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    }
}

/// Windows: `MPV_DIR` names an extracted libmpv development archive such as
/// shinchiro's `mpv-dev-x86_64-*.7z`: `include/mpv/client.h`, the runtime
/// `libmpv-2.dll`, and an import library. An MSVC `mpv.lib` is preferred when
/// present; otherwise the archive's `libmpv.dll.a` is linked verbatim. Its
/// members are COFF short-import records that MSVC `link.exe` and `lld-link`
/// accept directly, so no import library needs to be regenerated.
fn link_windows_mpv_dir(directory: &Path) {
    let directory = directory
        .canonicalize()
        .unwrap_or_else(|_| panic!("MPV_DIR {} does not exist", directory.display()));
    let header = directory.join("include/mpv/client.h");
    let text = std::fs::read_to_string(&header)
        .unwrap_or_else(|_| panic!("MPV_DIR is missing {}", header.display()));
    let version = client_api_version(&text)
        .unwrap_or_else(|| panic!("{} has no MPV_CLIENT_API_VERSION", header.display()));
    assert!(
        version >= MIN_CLIENT_API,
        "MPV_DIR provides libmpv client API {}.{}; {}.{} or newer is required",
        version.0,
        version.1,
        MIN_CLIENT_API.0,
        MIN_CLIENT_API.1
    );
    println!("cargo:rerun-if-changed={}", header.display());
    let msvc_import = directory.join("mpv.lib");
    let gnu_import = directory.join("libmpv.dll.a");
    println!("cargo:rustc-link-search=native={}", directory.display());
    if msvc_import.is_file() {
        println!("cargo:rerun-if-changed={}", msvc_import.display());
        println!("cargo:rustc-link-lib=dylib=mpv");
    } else if gnu_import.is_file() {
        println!("cargo:rerun-if-changed={}", gnu_import.display());
        println!("cargo:rustc-link-lib=dylib:+verbatim=libmpv.dll.a");
    } else {
        panic!(
            "MPV_DIR {} has neither mpv.lib nor libmpv.dll.a",
            directory.display()
        );
    }
    // Cargo prepends link-search directories inside the target directory to
    // PATH for `cargo run`/`cargo test`, so a private copy of the runtime DLL
    // makes development runs work without editing the user's PATH. Packages
    // must still ship libmpv-2.dll next to oxplay.exe.
    if let Some(dll) = runtime_dll(&directory) {
        println!("cargo:rerun-if-changed={}", dll.display());
        let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
        let copy = out.join(dll.file_name().expect("DLL file name"));
        std::fs::copy(&dll, &copy)
            .unwrap_or_else(|_| panic!("copy {} into OUT_DIR", dll.display()));
        println!("cargo:rustc-link-search=native={}", out.display());
    } else {
        println!("cargo:warning=MPV_DIR has no libmpv-2.dll; put it on PATH to run Oxplay");
    }
}

fn runtime_dll(directory: &Path) -> Option<PathBuf> {
    ["libmpv-2.dll", "mpv-2.dll"]
        .into_iter()
        .map(|name| directory.join(name))
        .find(|path| path.is_file())
}

/// Parses `#define MPV_CLIENT_API_VERSION MPV_MAKE_VERSION(major, minor)`.
fn client_api_version(header: &str) -> Option<(u32, u32)> {
    let line = header
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("#define MPV_CLIENT_API_VERSION"))?;
    let arguments = line.split_once("MPV_MAKE_VERSION(")?.1.split_once(')')?.0;
    let (major, minor) = arguments.split_once(',')?;
    Some((major.trim().parse().ok()?, minor.trim().parse().ok()?))
}
