use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, ensure};
use futures_lite::io::AsyncReadExt as _;
use gpui::http_client::{AsyncBody, HttpClient};
use semver::Version;
use serde::{Deserialize, Serialize};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const UPDATE_MANIFEST_URL: &str =
    "https://github.com/yuWorm/yttt/releases/latest/download/update.json";
pub const AUTO_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_MANIFEST_BYTES: u64 = 512 * 1024;
const RELEASES_URL_PREFIX: &str = "https://github.com/yuWorm/yttt/releases/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateAsset {
    pub url: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: Version,
    pub release_url: String,
    pub notes: String,
    pub asset: Option<UpdateAsset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateCheck {
    UpToDate,
    Available(UpdateInfo),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateCheckCache {
    pub last_checked_unix_seconds: Option<u64>,
    pub last_notified_version: Option<String>,
}

impl UpdateCheckCache {
    pub fn is_due(&self, now: SystemTime) -> bool {
        let Ok(now) = now.duration_since(UNIX_EPOCH) else {
            return true;
        };
        self.last_checked_unix_seconds.is_none_or(|last_checked| {
            now.as_secs().saturating_sub(last_checked) >= AUTO_CHECK_INTERVAL.as_secs()
        })
    }

    pub fn record_checked(&mut self, now: SystemTime) {
        self.last_checked_unix_seconds = now
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|elapsed| elapsed.as_secs());
    }

    pub fn should_notify(&self, version: &Version) -> bool {
        self.last_notified_version.as_deref() != Some(version.to_string().as_str())
    }

    pub fn record_notified(&mut self, version: &Version) {
        self.last_notified_version = Some(version.to_string());
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateManifest {
    schema: u32,
    version: String,
    release_url: String,
    notes: String,
    assets: BTreeMap<String, UpdateManifestAsset>,
}

#[derive(Debug, Deserialize)]
struct UpdateManifestAsset {
    url: String,
    sha256: String,
}

pub async fn fetch_update(http_client: Arc<dyn HttpClient>) -> Result<UpdateCheck> {
    fetch_update_from_url(http_client, UPDATE_MANIFEST_URL, APP_VERSION).await
}

async fn fetch_update_from_url(
    http_client: Arc<dyn HttpClient>,
    manifest_url: &str,
    current_version: &str,
) -> Result<UpdateCheck> {
    let mut response = http_client
        .get(manifest_url, AsyncBody::empty(), true)
        .await
        .context("failed to request the update manifest")?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .body_mut()
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut body)
        .await
        .context("failed to read the update manifest")?;
    ensure!(
        body.len() as u64 <= MAX_MANIFEST_BYTES,
        "update manifest exceeds {MAX_MANIFEST_BYTES} bytes"
    );
    ensure!(
        status.is_success(),
        "update manifest request failed with {status}: {}",
        String::from_utf8_lossy(&body)
    );

    evaluate_manifest(
        &body,
        current_version,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

fn evaluate_manifest(
    source: &[u8],
    current_version: &str,
    os: &str,
    arch: &str,
) -> Result<UpdateCheck> {
    let manifest: UpdateManifest =
        serde_json::from_slice(source).context("invalid update manifest JSON")?;
    ensure!(manifest.schema == 1, "unsupported update manifest schema");
    let current_version =
        Version::parse(current_version).context("invalid installed application version")?;
    let available_version =
        Version::parse(&manifest.version).context("invalid release version in update manifest")?;
    validate_release_url(&manifest.release_url, &available_version)?;

    if available_version <= current_version {
        return Ok(UpdateCheck::UpToDate);
    }

    let asset = if let Some(asset_key) = platform_asset_key(os, arch) {
        let asset = manifest
            .assets
            .get(asset_key)
            .with_context(|| format!("update manifest has no {asset_key} asset"))?;
        validate_asset(asset, &available_version)?;
        Some(UpdateAsset {
            url: asset.url.clone(),
            sha256: asset.sha256.clone(),
        })
    } else {
        None
    };

    Ok(UpdateCheck::Available(UpdateInfo {
        version: available_version,
        release_url: manifest.release_url,
        notes: manifest.notes,
        asset,
    }))
}

fn validate_release_url(url: &str, version: &Version) -> Result<()> {
    ensure!(
        url == format!("{RELEASES_URL_PREFIX}tag/v{version}"),
        "update manifest has an unexpected release URL"
    );
    Ok(())
}

fn validate_asset(asset: &UpdateManifestAsset, version: &Version) -> Result<()> {
    let expected_prefix = format!("{RELEASES_URL_PREFIX}download/v{version}/");
    ensure!(
        asset.url.starts_with(&expected_prefix),
        "update manifest has an unexpected asset URL"
    );
    ensure!(
        asset.sha256.len() == 64
            && asset
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "update manifest has an invalid SHA-256 digest"
    );
    Ok(())
}

fn platform_asset_key(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

pub fn load_update_cache(path: &Path) -> Result<UpdateCheckCache> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UpdateCheckCache::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    toml::from_str(&source).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn save_update_cache(path: &Path, cache: &UpdateCheckCache) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let source = toml::to_string_pretty(cache).context("failed to serialize update state")?;
    fs::write(path, source).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> Vec<u8> {
        format!(
            r#"{{
  "schema": 1,
  "version": "{version}",
  "releaseUrl": "https://github.com/yuWorm/yttt/releases/tag/v{version}",
  "notes": "A useful release.",
  "assets": {{
    "macos-aarch64": {{
      "url": "https://github.com/yuWorm/yttt/releases/download/v{version}/yttt-{version}-macos-aarch64.dmg",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }},
    "windows-x86_64": {{
      "url": "https://github.com/yuWorm/yttt/releases/download/v{version}/yttt-{version}-windows-x86_64-setup.exe",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    }},
    "linux-x86_64": {{
      "url": "https://github.com/yuWorm/yttt/releases/download/v{version}/yttt-{version}-linux-x86_64.tar.gz",
      "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
    }}
  }}
}}"#
        )
        .into_bytes()
    }

    #[test]
    fn newer_manifest_selects_the_current_platform_asset() {
        let check = evaluate_manifest(&manifest("1.1.0"), "1.0.0", "windows", "x86_64")
            .expect("manifest should be accepted");
        let UpdateCheck::Available(update) = check else {
            panic!("expected an available update");
        };
        assert_eq!(update.version, Version::new(1, 1, 0));
        assert!(
            update
                .asset
                .expect("Windows asset should exist")
                .url
                .ends_with("windows-x86_64-setup.exe")
        );
    }

    #[test]
    fn equal_or_older_manifests_are_up_to_date() {
        for version in ["1.0.0", "0.9.9"] {
            assert_eq!(
                evaluate_manifest(&manifest(version), "1.0.0", "linux", "x86_64").unwrap(),
                UpdateCheck::UpToDate
            );
        }
    }

    #[test]
    fn unsupported_platform_falls_back_to_the_release_page() {
        let UpdateCheck::Available(update) =
            evaluate_manifest(&manifest("1.1.0"), "1.0.0", "linux", "aarch64").unwrap()
        else {
            panic!("expected an available update");
        };
        assert!(update.asset.is_none());
    }

    #[test]
    fn manifest_rejects_untrusted_asset_urls_and_digests() {
        let source = String::from_utf8(manifest("1.1.0"))
            .unwrap()
            .replace(
                "https://github.com/yuWorm/yttt/releases/download/v1.1.0/yttt-1.1.0-linux-x86_64.tar.gz",
                "https://example.com/yttt.tar.gz",
            );
        assert!(evaluate_manifest(source.as_bytes(), "1.0.0", "linux", "x86_64").is_err());

        let source = String::from_utf8(manifest("1.1.0"))
            .unwrap()
            .replace(&"c".repeat(64), "not-a-digest");
        assert!(evaluate_manifest(source.as_bytes(), "1.0.0", "linux", "x86_64").is_err());
    }

    #[test]
    fn cache_enforces_the_daily_check_interval() {
        let now = UNIX_EPOCH + Duration::from_secs(100_000);
        let mut cache = UpdateCheckCache::default();
        assert!(cache.is_due(now));
        cache.record_checked(now);
        assert!(!cache.is_due(now + AUTO_CHECK_INTERVAL - Duration::from_secs(1)));
        assert!(cache.is_due(now + AUTO_CHECK_INTERVAL));
    }
}
