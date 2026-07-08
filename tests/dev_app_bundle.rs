#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(target_os = "macos")]
static DEV_APP_BUNDLE_SCRIPT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
#[cfg(target_os = "macos")]
fn dev_app_bundle_script_generates_bundle_without_opening() {
    let _guard = DEV_APP_BUNDLE_SCRIPT_LOCK.lock().unwrap();
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
    let bundled_binary = bundle_path.join("Contents/MacOS/yttt-bin");
    let fixture_shell = bundle_path.join("Contents/MacOS/yttt-fixture-shell");
    let launcher = bundle_path.join("Contents/MacOS/yttt-launcher");
    let launcher_source = std::fs::read_to_string(&launcher).unwrap();
    let fixture_shell_source = std::fs::read_to_string(&fixture_shell).unwrap();
    let script_source =
        std::fs::read_to_string(repo.join("scripts/run-dev-app.sh")).expect("read dev app script");

    assert!(plist.contains("com.yttt.dev"));
    assert!(plist.contains("CFBundleIconFile"));
    assert!(icon.exists());
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
    assert!(script_source.contains("open -n \"$bundle_dir\""));
}

#[test]
#[cfg(target_os = "macos")]
fn dev_app_bundle_none_fixture_preserves_user_shell() {
    let _guard = DEV_APP_BUNDLE_SCRIPT_LOCK.lock().unwrap();
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
