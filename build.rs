const WINDOWS_ICON: &str = "assets/app-icon/windows/AppIcon.ico";

fn main() {
    println!("cargo::rerun-if-changed={WINDOWS_ICON}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(WINDOWS_ICON)
        .set("FileDescription", "yttt terminal workbench")
        .set("ProductName", "yttt")
        .set("OriginalFilename", "yttt.exe");
    resource
        .compile()
        .expect("failed to compile Windows application resources");
}
