fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .with_style("fluent".into())
            // Only development UI tests need element introspection metadata.
            // Production keeps the same compiled components without that data.
            .with_debug_info(std::env::var("PROFILE").is_ok_and(|profile| profile == "debug")),
    )
    .expect("compile shared UI");
}
