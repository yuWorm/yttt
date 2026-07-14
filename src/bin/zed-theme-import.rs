use std::{env, path::PathBuf, process::ExitCode};

use anyhow::{Context as _, Result, bail};
use yttt::{
    config::paths::AppConfigPaths,
    ui::theme::zed::{detect_zed_theme_extension, import_detected_zed_themes_to},
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zed-theme-import: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let Some(extension_dir) = args.next() else {
        bail!("usage: zed-theme-import <zed-extension-directory> [output-directory]");
    };
    let output_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| AppConfigPaths::for_app().themes_dir());
    if args.next().is_some() {
        bail!("usage: zed-theme-import <zed-extension-directory> [output-directory]");
    }

    let extension_dir = PathBuf::from(extension_dir);
    let detection = detect_zed_theme_extension(&extension_dir);
    for warning in &detection.warnings {
        eprintln!("Warning: {}: {}", warning.path.display(), warning.message);
    }
    if detection.is_empty() {
        bail!(
            "no compatible UI or icon themes found in {}",
            extension_dir.display()
        );
    }
    for extension in &detection.extensions {
        for theme in &extension.ui_theme_names {
            println!("Detected UI theme {:?} ({})", theme, extension.name);
        }
        for theme in &extension.icon_theme_names {
            println!("Detected icon theme {:?} ({})", theme, extension.name);
        }
    }

    let imported = import_detected_zed_themes_to(&detection, &output_dir, output_dir.join("icons"))
        .with_context(|| format!("failed to import {}", extension_dir.display()))?;
    for theme in imported.ui_themes {
        println!(
            "Imported UI theme {:?} to {}",
            theme.theme_name,
            theme.path.display()
        );
    }
    for theme in imported.icon_themes {
        println!(
            "Imported icon themes {:?} to {}",
            theme.theme_names,
            theme.path.display()
        );
    }
    Ok(())
}
