fn main() {
    slint_build::compile("ui/app-window.slint").expect("failed to compile Slint UI");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/app.ico");
        resource
            .compile()
            .expect("failed to embed Windows resources");
    }
}
