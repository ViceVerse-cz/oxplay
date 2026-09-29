fn main() {
    println!("cargo:rerun-if-changed=src/native_child.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/native_child.m")
            .flag("-fobjc-arc")
            .flag("-Wno-deprecated-declarations")
            .compile("serein_native_child");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=OpenGL");
    }
    pkg_config::Config::new()
        .atleast_version("2.5")
        .probe("mpv")
        .expect("libmpv development files required (macOS: brew install mpv; Linux: libmpv-dev)");
}
