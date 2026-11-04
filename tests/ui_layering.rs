//! UI code reaches the component layer through `yttt-ui`. Only the theme
//! bridge and the editor's syntax engine still name `gpui_component` directly.

use std::{fs, path::Path};

const ALLOWED: &[&str] = &["src/ui/theme/", "src/ui/editor/"];

#[test]
fn only_the_theme_bridge_and_editor_name_gpui_component() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    collect_offenders(&root.join("src"), root, &mut offenders);
    offenders.sort();
    assert!(
        offenders.is_empty(),
        "use yttt_ui instead of gpui_component in: {offenders:#?}"
    );
}

fn collect_offenders(dir: &Path, root: &Path, offenders: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_offenders(&path, root, offenders);
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if ALLOWED.iter().any(|prefix| relative.starts_with(prefix)) {
            continue;
        }
        if fs::read_to_string(&path)
            .unwrap()
            .contains("gpui_component")
        {
            offenders.push(relative);
        }
    }
}
