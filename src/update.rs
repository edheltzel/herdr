//! Self-update mechanism.
//!
//! Checks GitHub releases for newer versions and downloads the binary.
//! Uses `curl` as a subprocess for HTTP — no additional Rust HTTP dependencies.
//! JSON parsing uses serde_json (already in deps for persistence).

use std::env;
use std::fs;
use std::process::Command;

use serde::Deserialize;

const GITHUB_REPO: &str = "ogulcancelik/herdr";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const FAKE_UPDATE_VERSION_ENV: &str = "HERDR_FAKE_UPDATE_VERSION";

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// Parsed semver version for comparison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        Some(Self {
            major: parts[0].parse().ok()?,
            minor: parts[1].parse().ok()?,
            patch: parts[2].parse().ok()?,
        })
    }

    pub fn current() -> Self {
        Self::parse(CURRENT_VERSION).expect("invalid CARGO_PKG_VERSION")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

// ---------------------------------------------------------------------------
// GitHub release API types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

// ---------------------------------------------------------------------------
// Release info
// ---------------------------------------------------------------------------

/// Information about an available update.
struct ReleaseInfo {
    version: Version,
    download_url: String,
}

/// Check GitHub for the latest release. Returns release info if newer.
fn check_latest() -> Result<Option<ReleaseInfo>, String> {
    let current = Version::current();
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");

    let output = Command::new("curl")
        .args([
            "-sfL",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: herdr-updater",
            &url,
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    if !output.status.success() {
        return Err("failed to fetch release info".into());
    }

    let release: GitHubRelease = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse release JSON: {e}"))?;

    let latest = Version::parse(&release.tag_name)
        .ok_or_else(|| format!("invalid version tag: {}", release.tag_name))?;

    if latest <= current {
        return Ok(None); // up to date
    }

    // Find the asset for our platform
    let (os, arch) = platform_target();
    let asset_name = format!("herdr-{os}-{arch}");

    let download_url = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .map(|a| a.browser_download_url.clone())
        .ok_or_else(|| format!("no binary for {os}-{arch} in release {}", release.tag_name))?;

    Ok(Some(ReleaseInfo {
        version: latest,
        download_url,
    }))
}

// ---------------------------------------------------------------------------
// Download + install
// ---------------------------------------------------------------------------

/// Download and install a release. Returns the installed version.
fn download_and_install(release: &ReleaseInfo) -> Result<(), String> {
    let current_exe = env::current_exe().map_err(|e| format!("can't find current binary: {e}"))?;

    let parent = current_exe.parent().ok_or("can't find binary directory")?;

    // Check write permissions early
    let test_path = parent.join(".herdr-write-test");
    if let Err(e) = fs::write(&test_path, b"") {
        let _ = fs::remove_file(&test_path);
        return Err(format!(
            "install directory not writable: {} ({}). Try running with appropriate permissions.",
            parent.display(),
            e
        ));
    }
    let _ = fs::remove_file(&test_path);

    // Unique temp file (avoids races with concurrent instances)
    let tmp_path = parent.join(format!(".herdr-update-{}.tmp", std::process::id()));

    // Download the exact asset URL (pinned to the release we checked)
    let status = Command::new("curl")
        .args(["-sfL", "--max-time", "120", "-o"])
        .arg(&tmp_path)
        .arg(&release.download_url)
        .status()
        .map_err(|e| format!("download failed: {e}"))?;

    if !status.success() {
        let _ = fs::remove_file(&tmp_path);
        return Err("download failed".into());
    }

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o755)) {
            let _ = fs::remove_file(&tmp_path);
            return Err(format!("chmod failed: {e}"));
        }
    }

    // Atomic replace — rename over the current binary.
    // On Linux, the running process keeps its fd to the old inode.
    // Next launch picks up the new file.
    if let Err(e) = fs::rename(&tmp_path, &current_exe) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("failed to replace binary: {e}"));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Manual self-update command (`herdr update`).
pub fn self_update() -> Result<Version, String> {
    eprintln!("checking for updates...");

    let current = Version::current();

    let release = match check_latest()? {
        Some(r) => r,
        None => {
            eprintln!("already up to date (v{current})");
            return Ok(current);
        }
    };

    eprintln!("downloading v{}...", release.version);
    download_and_install(&release)?;
    eprintln!("updated to v{}", release.version);

    Ok(release.version)
}

/// Background auto-update: check, download, install, notify TUI.
/// Runs in a background thread at startup.
pub fn auto_update(events: tokio::sync::mpsc::Sender<crate::events::AppEvent>) {
    if let Ok(version) = env::var(FAKE_UPDATE_VERSION_ENV) {
        let version = version.trim();
        if !version.is_empty() {
            tracing::info!(
                env = FAKE_UPDATE_VERSION_ENV,
                version,
                "using fake update version for local testing"
            );
            let _ = events.blocking_send(crate::events::AppEvent::UpdateReady {
                version: version.to_string(),
            });
        }
        return;
    }

    let release = match check_latest() {
        Ok(Some(r)) => r,
        _ => return, // up to date or failed — silently do nothing
    };

    tracing::info!(
        "new version v{} available, downloading from {}",
        release.version,
        release.download_url
    );

    if let Err(e) = download_and_install(&release) {
        tracing::warn!("auto-update failed: {e}");
        return;
    }

    tracing::info!(
        "auto-update: v{} installed, restart to use",
        release.version
    );

    // Notify the TUI — blocking_send is safe from a std::thread
    let _ = events.blocking_send(crate::events::AppEvent::UpdateReady {
        version: release.version.to_string(),
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn platform_target() -> (&'static str, &'static str) {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unknown"
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    };

    (os, arch)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_basic() {
        assert_eq!(
            Version::parse("1.2.3"),
            Some(Version {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
    }

    #[test]
    fn parse_version_with_v_prefix() {
        assert_eq!(
            Version::parse("v0.1.0"),
            Some(Version {
                major: 0,
                minor: 1,
                patch: 0
            })
        );
    }

    #[test]
    fn parse_version_invalid() {
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::parse("abc"), None);
        assert_eq!(Version::parse(""), None);
    }

    #[test]
    fn version_ordering() {
        let v010 = Version::parse("0.1.0").unwrap();
        let v011 = Version::parse("0.1.1").unwrap();
        let v020 = Version::parse("0.2.0").unwrap();
        let v100 = Version::parse("1.0.0").unwrap();

        assert!(v010 < v011);
        assert!(v011 < v020);
        assert!(v020 < v100);
        assert!(v010 == Version::parse("0.1.0").unwrap());
    }

    #[test]
    fn version_display() {
        let v = Version {
            major: 0,
            minor: 1,
            patch: 0,
        };
        assert_eq!(v.to_string(), "0.1.0");
    }

    #[test]
    fn current_version_parses() {
        let v = Version::current();
        assert!(v.major < 100);
    }

    #[test]
    fn platform_target_is_known() {
        let (os, arch) = platform_target();
        assert!(os == "linux" || os == "macos", "os: {os}");
        assert!(arch == "x86_64" || arch == "aarch64", "arch: {arch}");
    }

    #[test]
    fn github_release_deserializes() {
        let json = r#"{
            "tag_name": "v0.2.0",
            "assets": [
                {"name": "herdr-linux-x86_64", "browser_download_url": "https://example.com/herdr-linux-x86_64"},
                {"name": "herdr-macos-aarch64", "browser_download_url": "https://example.com/herdr-macos-aarch64"}
            ]
        }"#;
        let release: GitHubRelease = serde_json::from_str(json).unwrap();
        assert_eq!(release.tag_name, "v0.2.0");
        assert_eq!(release.assets.len(), 2);
        assert_eq!(release.assets[0].name, "herdr-linux-x86_64");
    }

    #[test]
    fn github_release_ignores_extra_fields() {
        let json = r#"{
            "tag_name": "v0.1.0",
            "name": "Release v0.1.0",
            "body": "Some notes",
            "draft": false,
            "prerelease": false,
            "assets": []
        }"#;
        let release: GitHubRelease = serde_json::from_str(json).unwrap();
        assert_eq!(release.tag_name, "v0.1.0");
        assert!(release.assets.is_empty());
    }
}
