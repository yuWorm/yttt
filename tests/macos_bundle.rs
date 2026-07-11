#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt as _;

#[test]
#[cfg(target_os = "macos")]
fn macos_bundle_script_packages_a_signed_launchable_app() {
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir().unwrap();
    let bundle = temp.path().join("yttt.app");
    let source_binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_yttt"));
    let unsigned_bundle = temp.path().join("yttt-unsigned.app");
    let unsigned_output = std::process::Command::new(repo.join("scripts/build-macos-bundle.sh"))
        .arg("--no-build")
        .arg("--binary")
        .arg(&source_binary)
        .arg("--output")
        .arg(&unsigned_bundle)
        .arg("--no-sign")
        .current_dir(&repo)
        .output()
        .expect("run unsigned macOS bundle script");
    assert!(
        unsigned_output.status.success(),
        "unsigned script failed: {}",
        String::from_utf8_lossy(&unsigned_output.stderr)
    );
    assert_eq!(
        std::fs::read(unsigned_bundle.join("Contents/MacOS/yttt")).unwrap(),
        std::fs::read(&source_binary).unwrap(),
        "--binary must select the executable copied into the bundle"
    );

    let output = std::process::Command::new(repo.join("scripts/build-macos-bundle.sh"))
        .arg("--no-build")
        .arg("--binary")
        .arg(&source_binary)
        .arg("--output")
        .arg(&bundle)
        .arg("--print-bundle-path")
        .current_dir(&repo)
        .output()
        .expect("run macOS bundle script");

    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        bundle.to_string_lossy()
    );

    let plist = bundle.join("Contents/Info.plist");
    let bundled_binary = bundle.join("Contents/MacOS/yttt");
    let bundled_icon = bundle.join("Contents/Resources/AppIcon.icns");

    assert_eq!(plist_value(&plist, "CFBundleExecutable"), "yttt");
    assert_eq!(plist_value(&plist, "CFBundleIdentifier"), "com.yttt.app");
    assert_eq!(
        plist_value(&plist, "CFBundleShortVersionString"),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(plist_value(&plist, "CFBundlePackageType"), "APPL");
    assert_eq!(plist_value(&plist, "LSMinimumSystemVersion"), "13.0");
    assert!(bundled_binary.metadata().unwrap().len() > 0);
    assert!(
        bundled_binary.metadata().unwrap().permissions().mode() & 0o111 != 0,
        "bundled executable must retain execute permission"
    );
    assert_eq!(
        std::fs::read(&bundled_icon).unwrap(),
        std::fs::read(repo.join("assets/app-icon/macos/AppIcon.icns")).unwrap()
    );
    assert_eq!(
        std::fs::read(bundle.join("Contents/PkgInfo")).unwrap(),
        b"APPL????"
    );
    assert!(!bundle.join("Contents/MacOS/yttt-launcher").exists());

    let signature = std::process::Command::new("/usr/bin/codesign")
        .arg("--verify")
        .arg("--deep")
        .arg("--strict")
        .arg(&bundle)
        .output()
        .expect("verify bundle signature");
    assert!(
        signature.status.success(),
        "bundle signature is invalid: {}",
        String::from_utf8_lossy(&signature.stderr)
    );
}

#[cfg(target_os = "macos")]
fn plist_value(plist: &std::path::Path, key: &str) -> String {
    let output = std::process::Command::new("/usr/bin/plutil")
        .arg("-extract")
        .arg(key)
        .arg("raw")
        .arg("-o")
        .arg("-")
        .arg(plist)
        .output()
        .expect("read plist value");
    assert!(
        output.status.success(),
        "failed to read {key}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}
