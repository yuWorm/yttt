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
    let launch_agent = bundle.join("Contents/Library/LaunchAgents/com.yttt.host.plist");

    assert_eq!(plist_value(&plist, "CFBundleExecutable"), "yttt");
    assert_eq!(plist_value(&plist, "CFBundleIdentifier"), "com.yttt.app");
    assert_eq!(
        plist_value(&plist, "CFBundleShortVersionString"),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(plist_value(&plist, "CFBundlePackageType"), "APPL");
    assert_eq!(plist_value(&plist, "CFBundleIconFile"), "AppIcon.icns");
    assert!(
        !std::fs::read_to_string(&plist)
            .unwrap()
            .contains("CFBundleIconName")
    );
    assert_eq!(plist_value(&plist, "LSMinimumSystemVersion"), "13.0");
    assert_eq!(plist_value(&launch_agent, "Label"), "com.yttt.host");
    assert_eq!(
        plist_value(&launch_agent, "BundleProgram"),
        "Contents/MacOS/yttt"
    );
    let launch_agent_source = std::fs::read_to_string(&launch_agent).unwrap();
    for argument in ["--start-host", "--profile-id", "default"] {
        assert!(
            launch_agent_source.contains(&format!("<string>{argument}</string>")),
            "LaunchAgent must preserve the stable Host startup argument {argument}"
        );
    }
    for secret_name in ["token", "password", "credential", "auth"] {
        assert!(
            !launch_agent_source
                .to_ascii_lowercase()
                .contains(secret_name),
            "LaunchAgent must not persist {secret_name} material"
        );
    }
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

#[test]
#[cfg(target_os = "macos")]
fn macos_bundle_reopens_and_reattaches_a_persistent_host_terminal() {
    use yttt::{
        config::{
            paths::AppConfigPaths,
            profile::{
                AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence,
                ProjectConfigPolicy,
            },
        },
        host_launcher::HostLauncher,
        model::ids::ProfileId,
    };
    use yttt_protocol::{Request, Response};

    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temporary = tempfile::tempdir().unwrap();
    let bundle = temporary.path().join("yttt-reattach.app");
    let source_binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_yttt"));
    let bundle_output = std::process::Command::new(repo.join("scripts/build-macos-bundle.sh"))
        .arg("--no-build")
        .arg("--binary")
        .arg(&source_binary)
        .arg("--output")
        .arg(&bundle)
        .current_dir(&repo)
        .output()
        .expect("build reattach smoke bundle");
    assert!(
        bundle_output.status.success(),
        "bundle script failed: {}",
        String::from_utf8_lossy(&bundle_output.stderr)
    );
    let bundled_binary = bundle.join("Contents/MacOS/yttt");

    let profile_root = temporary.path().join("profile");
    let project = temporary.path().join("project");
    std::fs::create_dir_all(profile_root.join("config")).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        profile_root.join("config/settings.toml"),
        "[general]\nonboarding_completed = true\n\n[window]\neffect = \"none\"\nopacity = 1.0\n",
    )
    .unwrap();
    let profile = AppProfile::scoped(
        ProfileId::new("performance"),
        EnvironmentKind::Development,
        ProfilePersistence::Ephemeral,
        &profile_root,
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ProfileDiscovery,
    );
    let layout_path = AppConfigPaths::from_profile(&profile).project_layout_file(&project);
    std::fs::create_dir_all(layout_path.parent().unwrap()).unwrap();
    std::fs::write(
        layout_path,
        format!(
            concat!(
                "[project]\n",
                "name = \"Bundle Reattach\"\n",
                "default_tab = \"dev\"\n\n",
                "[[tabs]]\n",
                "id = \"dev\"\n",
                "title = \"Dev\"\n",
                "startup = \"eager\"\n",
                "cwd = {cwd:?}\n",
                "layout = {{ type = \"pane\", id = \"shell\", title = \"Shell\", ",
                "command = \"/bin/sh\", args = [\"-lc\", \"while :; do sleep 60; done\"], ",
                "execution_mode = \"command\", exit_behavior = \"manual_restart\" }}\n",
            ),
            cwd = project.to_string_lossy(),
        ),
    )
    .unwrap();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let launcher = HostLauncher::new(profile.clone(), bundled_binary.clone());
    let host = runtime.block_on(launcher.launch_or_attach()).unwrap();
    assert!(host.spawned(), "bundle smoke must own the Host child");
    let diagnostics = profile.paths().runtime.join("host-diagnostics.jsonl");
    let mut desktops = Vec::new();
    let smoke = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        desktops.push(start_bundled_desktop(
            &bundled_binary,
            &profile_root,
            &project,
        ));
        wait_for_bundle_state("first desktop attachment", || {
            latest_bundle_diagnostics(&diagnostics)
                .is_some_and(|(sessions, attachments)| sessions == 1 && attachments >= 1)
        });
        let original_session = runtime.block_on(async {
            let mut client = host.connect().await.unwrap();
            let Response::Resources(catalog) =
                client.request(Request::ListResources).await.unwrap()
            else {
                panic!("Host returned a non-catalog response");
            };
            assert_eq!(catalog.terminals.len(), 1);
            catalog.terminals[0].session_id.clone()
        });

        desktops[0].kill().unwrap();
        desktops[0].wait().unwrap();
        wait_for_bundle_state("desktop detach with Host survival", || {
            latest_bundle_diagnostics(&diagnostics)
                .is_some_and(|(sessions, attachments)| sessions == 1 && attachments == 0)
        });

        desktops.push(start_bundled_desktop(
            &bundled_binary,
            &profile_root,
            &project,
        ));
        wait_for_bundle_state("reopened desktop attachment", || {
            latest_bundle_diagnostics(&diagnostics)
                .is_some_and(|(sessions, attachments)| sessions == 1 && attachments >= 1)
        });
        let reattached_session = runtime.block_on(async {
            let mut client = host.connect().await.unwrap();
            let Response::Resources(catalog) =
                client.request(Request::ListResources).await.unwrap()
            else {
                panic!("Host returned a non-catalog response after reopen");
            };
            assert_eq!(catalog.terminals.len(), 1);
            catalog.terminals[0].session_id.clone()
        });
        assert_eq!(reattached_session, original_session);
    }));

    for desktop in &mut desktops {
        let _ = desktop.kill();
        let _ = desktop.wait();
    }
    let stopped = runtime.block_on(host.drain_and_stop());
    match smoke {
        Ok(()) => stopped.unwrap(),
        Err(panic) => {
            let _ = stopped;
            std::panic::resume_unwind(panic);
        }
    }
}

#[cfg(target_os = "macos")]
fn start_bundled_desktop(
    binary: &std::path::Path,
    profile_root: &std::path::Path,
    project: &std::path::Path,
) -> std::process::Child {
    std::process::Command::new(binary)
        .arg("--project")
        .arg(project)
        .env("YTTT_PROFILE_ROOT", profile_root)
        .spawn()
        .expect("launch bundled desktop")
}

#[cfg(target_os = "macos")]
fn wait_for_bundle_state(label: &str, mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("timed out waiting for {label}");
}

#[cfg(target_os = "macos")]
fn latest_bundle_diagnostics(path: &std::path::Path) -> Option<(u64, u64)> {
    let content = std::fs::read_to_string(path).ok()?;
    let snapshot = content
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())?;
    Some((
        snapshot.get("sessions")?.as_u64()?,
        snapshot.get("attachments")?.as_u64()?,
    ))
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
