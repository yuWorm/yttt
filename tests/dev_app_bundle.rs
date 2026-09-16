#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(target_os = "macos")]
static DEV_APP_BUNDLE_SCRIPT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
#[cfg(target_os = "macos")]
fn dev_app_bundle_script_generates_bundle_without_opening() {
    let _guard = DEV_APP_BUNDLE_SCRIPT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = std::process::Command::new(repo.join("scripts/run-dev-app.sh"))
        .arg("--no-open")
        .arg("--fixture")
        .arg("dev")
        .arg("--print-bundle-path")
        .current_dir(&repo)
        .output()
        .expect("run dev app script");

    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bundle_path = String::from_utf8(output.stdout).unwrap();
    let bundle_path = repo.join(bundle_path.trim());
    let plist = std::fs::read_to_string(bundle_path.join("Contents/Info.plist")).unwrap();
    let icon = bundle_path.join("Contents/Resources/AppIcon.icns");
    let source_icon = repo.join("assets/app-icon/macos/AppIcon.icns");
    let bundled_binary = bundle_path.join("Contents/MacOS/yttt-bin");
    let fixture_shell = bundle_path.join("Contents/MacOS/yttt-fixture-shell");
    let launcher = bundle_path.join("Contents/MacOS/yttt-launcher");

    assert!(plist.contains("com.yttt.dev"));
    assert!(plist.contains("<key>CFBundleIconFile</key>"));
    assert!(plist.contains("<string>AppIcon.icns</string>"));
    assert!(plist.contains("NSPrincipalClass"));
    assert!(icon.exists());
    assert_eq!(
        std::fs::read(&icon).unwrap(),
        std::fs::read(source_icon).unwrap()
    );
    assert!(bundled_binary.exists());
    assert!(bundled_binary.metadata().unwrap().permissions().mode() & 0o111 != 0);
    assert!(fixture_shell.metadata().unwrap().permissions().mode() & 0o111 != 0);
    let shell_output = std::process::Command::new(&fixture_shell)
        .args(["-lc", "printf '%s' 'yttt fixture shell'"])
        .output()
        .expect("run bundled fixture shell");
    assert!(shell_output.status.success());
    assert_eq!(shell_output.stdout, b"yttt fixture shell");
    assert!(launcher.metadata().unwrap().permissions().mode() & 0o111 != 0);
}
