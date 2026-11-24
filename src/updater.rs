use crossbeam_channel::{unbounded, Receiver, Sender};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[allow(dead_code)]
pub const IS_PRIVATE_BUILD: bool = false;

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/Ilay-dev/Rustify/releases/latest";
const MIN_INSTALLER_BYTES: u64 = 500_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateReason {
    NewerVersion,
    SizeChanged,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct UpdateInfo {
    pub version: String,
    pub tag: String,
    pub asset_name: String,
    pub download_url: String,
    pub html_url: String,
    pub asset_size: u64,
    pub sha256: Option<String>,
    pub notes: String,
    pub reason: UpdateReason,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate {
        latest_version: String,
        asset_size: u64,
    },
    Available(UpdateInfo),
    Downloading {
        info: UpdateInfo,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    ReadyToInstall,
    Error(String),
}

#[derive(Debug, Default)]
pub struct UpdatePollResult {
    pub open_startup_popup: bool,
    pub save_baseline_size: Option<u64>,
    pub should_exit_for_install: bool,
    pub error_toast: Option<String>,
}

enum UpdateEvent {
    UpToDate {
        latest_version: String,
        asset_size: u64,
        save_baseline: bool,
    },
    Available {
        info: UpdateInfo,
        is_startup: bool,
    },
    DownloadProgress {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    ReadyToInstall,
    Error {
        message: String,
        is_startup: bool,
    },
}

pub fn parse_semver(s: &str) -> (u32, u32, u32) {
    let clean = s.trim().trim_start_matches(['v', 'V']);
    let mut parts = clean.split('.');
    let parse_part = |opt: Option<&str>| -> u32 {
        opt.and_then(|p| p.split(['-', '+']).next())
            .and_then(|p| p.trim().parse::<u32>().ok())
            .unwrap_or(0)
    };
    let major = parse_part(parts.next());
    let minor = parse_part(parts.next());
    let patch = parse_part(parts.next());
    (major, minor, patch)
}

pub fn detect_update(
    current_ver: &str,
    remote_tag: &str,
    remote_size: u64,
    last_known_size: u64,
) -> Option<UpdateReason> {
    let cur = parse_semver(current_ver);
    let rem = parse_semver(remote_tag);
    if rem > cur {
        Some(UpdateReason::NewerVersion)
    } else if rem == cur && remote_size > 0 && last_known_size > 0 && remote_size != last_known_size
    {
        Some(UpdateReason::SizeChanged)
    } else {
        None
    }
}

pub fn format_bytes_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn clamp_release_notes(raw: &str, max_chars: usize) -> String {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cleaned = trimmed
            .trim_start_matches('#')
            .trim()
            .replace("**", "");
        if !cleaned.is_empty() {
            lines.push(cleaned);
        }
    }
    let joined = lines.join("\n");
    if joined.chars().count() <= max_chars {
        return joined;
    }
    let mut out = String::new();
    for (idx, ch) in joined.chars().enumerate() {
        if idx >= max_chars.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

pub struct UpdateManager {
    status: UpdateStatus,
    tx: Sender<UpdateEvent>,
    rx: Receiver<UpdateEvent>,
}

impl UpdateManager {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        Self {
            status: UpdateStatus::Idle,
            tx,
            rx,
        }
    }

    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            self.status,
            UpdateStatus::Checking | UpdateStatus::Downloading { .. }
        )
    }

    pub fn check_now(&mut self, last_known_size: u64, is_startup: bool) {
        if matches!(
            self.status,
            UpdateStatus::Checking
                | UpdateStatus::Downloading { .. }
                | UpdateStatus::ReadyToInstall
        ) {
            return;
        }

        self.status = UpdateStatus::Checking;
        let tx = self.tx.clone();

        let _ = thread::Builder::new()
            .name("rustify-update-check".to_string())
            .spawn(move || {
                run_check_worker(tx, last_known_size, is_startup);
            });
    }

    pub fn start_download_and_install(&mut self, info: UpdateInfo) {
        if matches!(
            self.status,
            UpdateStatus::Downloading { .. } | UpdateStatus::ReadyToInstall
        ) {
            return;
        }

        self.status = UpdateStatus::Downloading {
            info: info.clone(),
            downloaded_bytes: 0,
            total_bytes: info.asset_size,
        };
        let tx = self.tx.clone();

        let _ = thread::Builder::new()
            .name("rustify-update-download".to_string())
            .spawn(move || {
                run_download_worker(tx, info);
            });
    }

    pub fn poll(&mut self) -> UpdatePollResult {
        let mut result = UpdatePollResult::default();
        for ev in self.rx.try_iter() {
            match ev {
                UpdateEvent::UpToDate {
                    latest_version,
                    asset_size,
                    save_baseline,
                } => {
                    if save_baseline && asset_size > 0 {
                        result.save_baseline_size = Some(asset_size);
                    }
                    self.status = UpdateStatus::UpToDate {
                        latest_version,
                        asset_size,
                    };
                }
                UpdateEvent::Available { info, is_startup } => {
                    self.status = UpdateStatus::Available(info);
                    if is_startup {
                        result.open_startup_popup = true;
                    }
                }
                UpdateEvent::DownloadProgress {
                    downloaded_bytes,
                    total_bytes,
                } => {
                    if let UpdateStatus::Downloading {
                        downloaded_bytes: ref mut dl,
                        total_bytes: ref mut tot,
                        ..
                    } = self.status
                    {
                        *dl = downloaded_bytes;
                        if total_bytes > 0 {
                            *tot = total_bytes;
                        }
                    }
                }
                UpdateEvent::ReadyToInstall => {
                    self.status = UpdateStatus::ReadyToInstall;
                    result.should_exit_for_install = true;
                }
                UpdateEvent::Error {
                    message,
                    is_startup,
                } => {
                    if is_startup {
                        self.status = UpdateStatus::Idle;
                    } else {
                        result.error_toast = Some(message.clone());
                        self.status = UpdateStatus::Error(message);
                    }
                }
            }
        }
        result
    }
}

fn is_allowed_github_download_url(url: &str) -> Result<&str, String> {
    let safe_url = crate::config::validate_http_url(url)?;
    if safe_url.starts_with("https://github.com/")
        || safe_url.starts_with("https://objects.githubusercontent.com/")
        || safe_url.starts_with("https://release-assets.githubusercontent.com/")
    {
        Ok(safe_url)
    } else {
        Err("Untrusted update download host".to_string())
    }
}

fn parse_sha256_digest(asset: &serde_json::Value) -> Option<String> {
    let raw = asset.get("digest")?.as_str()?.trim();
    let hex = raw.strip_prefix("sha256:")?;
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hex.to_ascii_lowercase())
    } else {
        None
    }
}

fn parse_sha256_from_body(body: &str) -> Option<String> {
    for token in body.split(|c: char| !c.is_ascii_hexdigit()) {
        if token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(token.to_ascii_lowercase());
        }
    }
    None
}

fn curl_bin() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "curl.exe"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "curl"
    }
}

#[cfg(any(not(target_os = "windows"), test))]
fn sanitize_asset_filename(raw_name: &str) -> String {
    let base = Path::new(raw_name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() || trimmed.contains("..") {
        "rustify-update.tar.gz".to_string()
    } else {
        trimmed.to_string()
    }
}

fn expected_magic_for_asset(asset_name: &str) -> Option<&'static [u8]> {
    #[cfg(target_os = "windows")]
    {
        let _ = asset_name;
        Some(b"MZ")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let lower = asset_name.to_ascii_lowercase();
        if lower.ends_with(".deb") {
            Some(b"!<arch>\n")
        } else if lower.ends_with(".pkg.tar.zst") {
            Some(&[0x28, 0xB5, 0x2F, 0xFD])
        } else if lower.ends_with(".tar.gz") {
            Some(&[0x1F, 0x8B])
        } else {
            None
        }
    }
}

fn verify_installer_file(path: &Path, asset_name: &str, expected_sha256: &str) -> bool {
    let Some(magic) = expected_magic_for_asset(asset_name) else {
        return false;
    };
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() < MIN_INSTALLER_BYTES {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65_536];
    let mut header = [0u8; 16];
    let mut header_len = 0usize;
    loop {
        let Ok(n) = file.read(&mut buf) else {
            return false;
        };
        if n == 0 {
            break;
        }
        if header_len < magic.len() {
            let take = (magic.len() - header_len).min(n);
            header[header_len..header_len + take].copy_from_slice(&buf[..take]);
            header_len += take;
            if header_len >= magic.len() && &header[..magic.len()] != magic {
                return false;
            }
        }
        hasher.update(&buf[..n]);
    }
    if header_len < magic.len() || &header[..magic.len()] != magic {
        return false;
    }
    let digest = hasher.finalize();
    let actual_hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    actual_hex == expected_sha256.to_ascii_lowercase()
}

fn find_asset_matching<F>(
    assets: &[serde_json::Value],
    raw_body: &str,
    predicate: F,
) -> Option<(String, String, u64, Option<String>)>
where
    F: Fn(&str) -> bool,
{
    for asset in assets {
        let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let url = asset
            .get("browser_download_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let size = asset.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        if predicate(name) && !url.is_empty() {
            let digest =
                parse_sha256_digest(asset).or_else(|| parse_sha256_from_body(raw_body));
            return Some((name.to_string(), url.to_string(), size, digest));
        }
    }
    None
}

fn select_release_asset(
    assets: &[serde_json::Value],
    raw_body: &str,
) -> Option<(String, String, u64, Option<String>)> {
    #[cfg(target_os = "windows")]
    {
        find_asset_matching(assets, raw_body, |n| {
            n.eq_ignore_ascii_case("Rustify-Setup.exe")
        })
        .or_else(|| {
            find_asset_matching(assets, raw_body, |n| {
                n.to_ascii_lowercase().ends_with(".exe")
            })
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        if Path::new("/etc/arch-release").exists() {
            find_asset_matching(assets, raw_body, |n| {
                n.to_ascii_lowercase().ends_with(".pkg.tar.zst")
            })
            .or_else(|| {
                find_asset_matching(assets, raw_body, |n| {
                    n.to_ascii_lowercase().ends_with("linux-x86_64.tar.gz")
                })
            })
        } else if Path::new("/etc/debian_version").exists() {
            find_asset_matching(assets, raw_body, |n| {
                n.to_ascii_lowercase().ends_with(".deb")
            })
            .or_else(|| {
                find_asset_matching(assets, raw_body, |n| {
                    n.to_ascii_lowercase().ends_with("linux-x86_64.tar.gz")
                })
            })
        } else {
            find_asset_matching(assets, raw_body, |n| {
                n.to_ascii_lowercase().ends_with("linux-x86_64.tar.gz")
            })
            .or_else(|| {
                find_asset_matching(assets, raw_body, |n| {
                    n.to_ascii_lowercase().ends_with(".deb")
                })
            })
            .or_else(|| {
                find_asset_matching(assets, raw_body, |n| {
                    n.to_ascii_lowercase().ends_with(".pkg.tar.zst")
                })
            })
        }
    }
}

fn run_check_worker(tx: Sender<UpdateEvent>, last_known_size: u64, is_startup: bool) {
    let Ok(api_url) = crate::config::validate_http_url(LATEST_RELEASE_API) else {
        let _ = tx.send(UpdateEvent::Error {
            message: "Invalid update API URL".to_string(),
            is_startup,
        });
        return;
    };

    let mut cmd = Command::new(curl_bin());
    cmd.args([
        "-sSL",
        "--max-time",
        "8",
        "-H",
        "User-Agent: Rustify-Updater",
        "-H",
        "Accept: application/vnd.github+json",
        "--",
        api_url,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = match cmd.output() {
        Ok(out) if out.status.success() => out.stdout,
        _ => {
            let _ = tx.send(UpdateEvent::Error {
                message: "Update check failed (network unreachable)".to_string(),
                is_startup,
            });
            return;
        }
    };

    let json: serde_json::Value = match serde_json::from_slice(&output) {
        Ok(v) => v,
        Err(_) => {
            let _ = tx.send(UpdateEvent::Error {
                message: "Update check failed (invalid release response)".to_string(),
                is_startup,
            });
            return;
        }
    };

    let Some(tag) = json.get("tag_name").and_then(|v| v.as_str()) else {
        let _ = tx.send(UpdateEvent::Error {
            message: "No published release found".to_string(),
            is_startup,
        });
        return;
    };

    let html_url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://github.com/Ilay-dev/Rustify/releases/latest")
        .to_string();

    let raw_body = json.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let notes = clamp_release_notes(raw_body, 320);

    let chosen_asset = json
        .get("assets")
        .and_then(|v| v.as_array())
        .and_then(|assets| select_release_asset(assets, raw_body));

    let (asset_name, download_url, asset_size, sha256) = chosen_asset.unwrap_or_default();
    let version = tag.trim().trim_start_matches(['v', 'V']).to_string();

    if let Some(reason) = detect_update(CURRENT_VERSION, tag, asset_size, last_known_size) {
        if download_url.is_empty() {
            let _ = tx.send(UpdateEvent::Error {
                message: "Release installer asset missing".to_string(),
                is_startup,
            });
            return;
        }
        if is_allowed_github_download_url(&download_url).is_err() {
            let _ = tx.send(UpdateEvent::Error {
                message: "Release installer URL is not from a trusted GitHub host".to_string(),
                is_startup,
            });
            return;
        }
        let info = UpdateInfo {
            version,
            tag: tag.to_string(),
            asset_name,
            download_url,
            html_url,
            asset_size,
            sha256,
            notes,
            reason,
        };
        let _ = tx.send(UpdateEvent::Available { info, is_startup });
    } else {
        let _ = tx.send(UpdateEvent::UpToDate {
            latest_version: version,
            asset_size,
            save_baseline: last_known_size == 0 && asset_size > 0,
        });
    }
}

fn run_download_worker(tx: Sender<UpdateEvent>, info: UpdateInfo) {
    let Ok(safe_url) = is_allowed_github_download_url(&info.download_url) else {
        let _ = tx.send(UpdateEvent::Error {
            message: "Update download blocked: untrusted URL".to_string(),
            is_startup: false,
        });
        return;
    };

    let Some(expected_sha256) = info.sha256.as_deref() else {
        let _ = tx.send(UpdateEvent::Error {
            message: "Update blocked: missing SHA-256 checksum in release".to_string(),
            is_startup: false,
        });
        return;
    };

    #[cfg(target_os = "windows")]
    let dest_path = std::env::temp_dir().join("Rustify-Update-Setup.exe");

    #[cfg(not(target_os = "windows"))]
    let (safe_name, dest_path) = {
        let safe_name = sanitize_asset_filename(&info.asset_name);
        let dl_dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let _ = fs::create_dir_all(&dl_dir);
        let dest = dl_dir.join(&safe_name);
        (safe_name, dest)
    };

    let _ = fs::remove_file(&dest_path);

    let mut cmd = Command::new(curl_bin());
    cmd.args([
        "-fSL",
        "--max-time",
        "180",
        "-o",
        &dest_path.to_string_lossy(),
        "--",
        safe_url,
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            let _ = tx.send(UpdateEvent::Error {
                message: "Failed to start update download".to_string(),
                is_startup: false,
            });
            return;
        }
    };

    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                let downloaded_bytes = fs::metadata(&dest_path).map(|m| m.len()).unwrap_or(0);
                let _ = tx.send(UpdateEvent::DownloadProgress {
                    downloaded_bytes,
                    total_bytes: info.asset_size,
                });
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => break Err(()),
        }
    };

    let ok_exit = matches!(exit_status, Ok(st) if st.success());
    if !ok_exit {
        let _ = fs::remove_file(&dest_path);
        let _ = tx.send(UpdateEvent::Error {
            message: "Update download failed".to_string(),
            is_startup: false,
        });
        return;
    }

    // Validate downloaded installer size, platform magic header, and SHA-256 checksum
    if !verify_installer_file(&dest_path, &info.asset_name, expected_sha256) {
        let _ = fs::remove_file(&dest_path);
        let _ = tx.send(UpdateEvent::Error {
            message: "Downloaded installer verification failed (size/header/SHA-256)".to_string(),
            is_startup: false,
        });
        return;
    }

    let final_bytes = fs::metadata(&dest_path)
        .map(|m| m.len())
        .unwrap_or(info.asset_size);
    let _ = tx.send(UpdateEvent::DownloadProgress {
        downloaded_bytes: final_bytes,
        total_bytes: final_bytes.max(info.asset_size),
    });

    #[cfg(target_os = "windows")]
    {
        // Launch installer directly without shell interpolation
        let mut launcher = Command::new(&dest_path);
        launcher
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if launcher.spawn().is_err() {
            let _ = tx.send(UpdateEvent::Error {
                message: "Failed to launch update installer".to_string(),
                is_startup: false,
            });
            return;
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let lower = safe_name.to_ascii_lowercase();
        let open_target: &Path = if lower.ends_with(".deb") {
            &dest_path
        } else {
            dest_path.parent().unwrap_or(&dest_path)
        };
        let mut launcher = Command::new("xdg-open");
        launcher
            .arg(open_target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if launcher.spawn().is_err() {
            let _ = tx.send(UpdateEvent::Error {
                message: format!(
                    "Update downloaded to {}, but failed to open via xdg-open",
                    dest_path.display()
                ),
                is_startup: false,
            });
            return;
        }
    }

    let _ = tx.send(UpdateEvent::ReadyToInstall);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_semver_variants() {
        assert_eq!(parse_semver("1.2.0"), (1, 2, 0));
        assert_eq!(parse_semver("v1.2.3"), (1, 2, 3));
        assert_eq!(parse_semver("V2.10.4-beta.1"), (2, 10, 4));
        assert_eq!(parse_semver("  v1.3  "), (1, 3, 0));
        assert_eq!(parse_semver("3.0.1+build.9"), (3, 0, 1));
        assert_eq!(parse_semver("invalid"), (0, 0, 0));
    }

    #[test]
    fn test_detect_update_newer_version() {
        assert_eq!(
            detect_update("1.2.0", "v1.2.1", 10_000_000, 10_000_000),
            Some(UpdateReason::NewerVersion)
        );
        assert_eq!(
            detect_update("1.2.0", "v1.3.0", 0, 0),
            Some(UpdateReason::NewerVersion)
        );
        assert_eq!(
            detect_update("1.2.0", "2.0.0", 12_000_000, 12_000_000),
            Some(UpdateReason::NewerVersion)
        );
    }

    #[test]
    fn test_detect_update_size_changed_same_version() {
        assert_eq!(
            detect_update("1.2.0", "v1.2.0", 14_500_000, 14_200_000),
            Some(UpdateReason::SizeChanged)
        );
        // No baseline yet -> do not spuriously trigger SizeChanged
        assert_eq!(detect_update("1.2.0", "v1.2.0", 14_500_000, 0), None);
        // Same size -> up to date
        assert_eq!(
            detect_update("1.2.0", "v1.2.0", 14_200_000, 14_200_000),
            None
        );
        // Older remote tag -> no update
        assert_eq!(
            detect_update("1.2.0", "v1.1.0", 13_000_000, 14_000_000),
            None
        );
    }

    #[test]
    fn test_allowed_github_download_url() {
        assert!(is_allowed_github_download_url(
            "https://github.com/Ilay-dev/Rustify/releases/download/v1.3.0/Rustify-Setup.exe"
        )
        .is_ok());
        assert!(is_allowed_github_download_url(
            "https://objects.githubusercontent.com/github-production-release-asset/foo.exe"
        )
        .is_ok());
        assert!(is_allowed_github_download_url(
            "https://release-assets.githubusercontent.com/github-production-release-asset/foo.exe"
        )
        .is_ok());
        assert!(is_allowed_github_download_url("https://evil.com/Rustify-Setup.exe").is_err());
        assert!(is_allowed_github_download_url("http://github.com/foo.exe").is_err());
        assert!(is_allowed_github_download_url("-o evil.exe").is_err());
    }

    #[test]
    fn test_parse_sha256_digest() {
        let valid = serde_json::json!({
            "digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789ABCDEF"
        });
        assert_eq!(
            parse_sha256_digest(&valid),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string())
        );
        let missing = serde_json::json!({"name": "Rustify-Setup.exe"});
        assert_eq!(parse_sha256_digest(&missing), None);
        let bad = serde_json::json!({"digest": "sha256:short"});
        assert_eq!(parse_sha256_digest(&bad), None);
        assert_eq!(
            parse_sha256_from_body(
                "Release notes\nSHA-256: FEDCBA9876543210FEDCBA9876543210FEDCBA9876543210FEDCBA9876543210"
            ),
            Some("fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210".to_string())
        );
    }

    #[test]
    fn test_sanitize_asset_filename() {
        assert_eq!(
            sanitize_asset_filename("rustify_1.4.0_amd64.deb"),
            "rustify_1.4.0_amd64.deb"
        );
        assert_eq!(
            sanitize_asset_filename("../../etc/evil.deb"),
            "evil.deb"
        );
        assert_eq!(sanitize_asset_filename(".."), "rustify-update.tar.gz");
        assert_eq!(sanitize_asset_filename(""), "rustify-update.tar.gz");
    }

    #[test]
    fn test_verify_installer_file_magic_and_sha256() {
        let (asset_name, magic): (&str, &[u8]) = {
            #[cfg(target_os = "windows")]
            {
                ("Rustify-Setup.exe", b"MZ")
            }
            #[cfg(not(target_os = "windows"))]
            {
                ("rustify_1.4.0_amd64.deb", b"!<arch>\n")
            }
        };

        let mut payload = vec![0x42u8; MIN_INSTALLER_BYTES as usize + 128];
        payload[..magic.len()].copy_from_slice(magic);

        let digest = Sha256::digest(&payload);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();

        let tmp = std::env::temp_dir().join(format!(
            "rustify_test_verify_{}_{}",
            std::process::id(),
            asset_name
        ));
        fs::write(&tmp, &payload).expect("write test installer");

        assert!(verify_installer_file(&tmp, asset_name, &hex));
        assert!(!verify_installer_file(
            &tmp,
            asset_name,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));

        // Corrupt magic header -> must fail even with matching sha256
        payload[0] ^= 0xFF;
        let bad_digest = Sha256::digest(&payload);
        let bad_hex: String = bad_digest.iter().map(|b| format!("{b:02x}")).collect();
        fs::write(&tmp, &payload).expect("write bad magic installer");
        assert!(!verify_installer_file(&tmp, asset_name, &bad_hex));

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_select_release_asset_for_current_os() {
        let assets = vec![
            serde_json::json!({
                "name": "Rustify-Setup.exe",
                "browser_download_url": "https://github.com/Ilay-dev/Rustify/releases/download/v1.4.0/Rustify-Setup.exe",
                "size": 14_000_000u64,
                "digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }),
            serde_json::json!({
                "name": "rustify_1.4.0_amd64.deb",
                "browser_download_url": "https://github.com/Ilay-dev/Rustify/releases/download/v1.4.0/rustify_1.4.0_amd64.deb",
                "size": 12_000_000u64,
                "digest": "sha256:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }),
            serde_json::json!({
                "name": "rustify-1.4.0-1-x86_64.pkg.tar.zst",
                "browser_download_url": "https://github.com/Ilay-dev/Rustify/releases/download/v1.4.0/rustify-1.4.0-1-x86_64.pkg.tar.zst",
                "size": 11_000_000u64,
                "digest": "sha256:2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }),
            serde_json::json!({
                "name": "rustify-1.4.0-linux-x86_64.tar.gz",
                "browser_download_url": "https://github.com/Ilay-dev/Rustify/releases/download/v1.4.0/rustify-1.4.0-linux-x86_64.tar.gz",
                "size": 12_500_000u64,
                "digest": "sha256:3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }),
        ];
        let picked = select_release_asset(&assets, "");
        assert!(picked.is_some());
        let (name, _url, size, sha) = picked.unwrap();
        assert!(size > 0);
        assert!(sha.is_some());
        #[cfg(target_os = "windows")]
        assert_eq!(name, "Rustify-Setup.exe");
        #[cfg(not(target_os = "windows"))]
        assert!(
            name.ends_with(".deb")
                || name.ends_with(".pkg.tar.zst")
                || name.ends_with("linux-x86_64.tar.gz")
        );
    }
}
