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
    let windows_icon = repo.join("assets/app-icon/windows/AppIcon.ico");
    let source_png = repo.join("assets/app-icon/source/yttt-icon.png");
    let bundled_binary = bundle_path.join("Contents/MacOS/yttt-bin");
    let fixture_shell = bundle_path.join("Contents/MacOS/yttt-fixture-shell");
    let launcher = bundle_path.join("Contents/MacOS/yttt-launcher");
    let launcher_source = std::fs::read_to_string(&launcher).unwrap();
    let fixture_shell_source = std::fs::read_to_string(&fixture_shell).unwrap();
    let script_source =
        std::fs::read_to_string(repo.join("scripts/run-dev-app.sh")).expect("read dev app script");
    let icon_script_source = std::fs::read_to_string(repo.join("scripts/build-app-icons.sh"))
        .expect("read app icon script");

    assert!(plist.contains("com.yttt.dev"));
    assert!(plist.contains("CFBundleIconFile"));
    assert!(plist.contains("CFBundleIconName"));
    assert!(plist.contains("NSPrincipalClass"));
    assert!(source_png.exists());
    assert!(source_icon.exists());
    assert!(windows_icon.exists());
    for size in ["16", "32", "48", "64", "128", "256", "512", "1024"] {
        assert!(
            repo.join(format!("assets/app-icon/png/{size}.png"))
                .exists()
        );
    }
    assert!(icon.exists());
    assert_eq!(
        std::fs::read(&icon).unwrap(),
        std::fs::read(source_icon).unwrap()
    );
    assert!(bundled_binary.exists());
    assert!(bundled_binary.metadata().unwrap().permissions().mode() & 0o111 != 0);
    assert!(fixture_shell.metadata().unwrap().permissions().mode() & 0o111 != 0);
    assert!(fixture_shell_source.contains("exec /bin/sh -c"));
    assert!(launcher.metadata().unwrap().permissions().mode() & 0o111 != 0);
    assert!(launcher_source.contains("bundle_rel=\"target/dev-app/yttt.app\""));
    assert!(launcher_source.contains("YTTT_DEV_FIXTURE=1"));
    assert!(launcher_source.contains("yttt-fixture-shell"));
    assert!(launcher_source.contains("yttt-bin"));
    assert!(script_source.contains("target/debug/yttt"));
    assert!(script_source.contains("assets/app-icon/macos/AppIcon.icns"));
    assert!(!script_source.contains("yttt-icon.svg"));
    assert!(!script_source.contains("GenericApplicationIcon.icns"));
    assert!(script_source.contains("open -n \"$bundle_dir\""));
    assert!(icon_script_source.contains("assets/app-icon/source/yttt-icon.png"));
    assert!(icon_script_source.contains("assets/app-icon/windows"));
    assert!(icon_script_source.contains("PNG32:"));
}

#[test]
#[cfg(target_os = "macos")]
fn dev_app_bundle_none_fixture_preserves_user_shell() {
    let _guard = DEV_APP_BUNDLE_SCRIPT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = std::process::Command::new(repo.join("scripts/run-dev-app.sh"))
        .arg("--no-open")
        .arg("--fixture")
        .arg("none")
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
    let launcher = bundle_path.join("Contents/MacOS/yttt-launcher");
    let launcher_source = std::fs::read_to_string(&launcher).unwrap();

    assert!(launcher_source.contains("unset YTTT_DEV_FIXTURE"));
    assert!(!launcher_source.contains("yttt-fixture-shell"));
}
