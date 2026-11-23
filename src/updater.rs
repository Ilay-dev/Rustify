use crossbeam_channel::{unbounded, Receiver, Sender};
use std::fs;
use std::io::Read;
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
    pub download_url: String,
    pub html_url: String,
    pub asset_size: u64,
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
    ReadyToInstall {
        installed_size: u64,
    },
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
                UpdateEvent::ReadyToInstall { installed_size } => {
                    if installed_size > 0 {
                        result.save_baseline_size = Some(installed_size);
                    }
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

fn run_check_worker(tx: Sender<UpdateEvent>, last_known_size: u64, is_startup: bool) {
    let mut cmd = Command::new("curl.exe");
    cmd.args([
        "-sSL",
        "--max-time",
        "8",
        "-H",
        "User-Agent: Rustify-Updater",
        "-H",
        "Accept: application/vnd.github+json",
        LATEST_RELEASE_API,
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

    let mut chosen_asset: Option<(String, u64)> = None;
    if let Some(assets) = json.get("assets").and_then(|v| v.as_array()) {
        // Prefer exact Rustify-Setup.exe first
        for asset in assets {
            let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let url = asset
                .get("browser_download_url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let size = asset.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            if name.eq_ignore_ascii_case("Rustify-Setup.exe") && !url.is_empty() {
                chosen_asset = Some((url.to_string(), size));
                break;
            }
        }
        // Fallback to any .exe asset
        if chosen_asset.is_none() {
            for asset in assets {
                let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let url = asset
                    .get("browser_download_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let size = asset.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                if name.to_ascii_lowercase().ends_with(".exe") && !url.is_empty() {
                    chosen_asset = Some((url.to_string(), size));
                    break;
                }
            }
        }
    }

    let (download_url, asset_size) = chosen_asset.unwrap_or_default();
    let version = tag.trim().trim_start_matches(['v', 'V']).to_string();

    if let Some(reason) = detect_update(CURRENT_VERSION, tag, asset_size, last_known_size) {
        if download_url.is_empty() {
            let _ = tx.send(UpdateEvent::Error {
                message: "Release installer asset missing".to_string(),
                is_startup,
            });
            return;
        }
        let info = UpdateInfo {
            version,
            tag: tag.to_string(),
            download_url,
            html_url,
            asset_size,
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
    let tmp_path = std::env::temp_dir().join("Rustify-Update-Setup.exe");
    let _ = fs::remove_file(&tmp_path);

    let mut cmd = Command::new("curl.exe");
    cmd.args([
        "-fSL",
        "--max-time",
        "180",
        "-o",
        &tmp_path.to_string_lossy(),
        &info.download_url,
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
                let downloaded_bytes = fs::metadata(&tmp_path).map(|m| m.len()).unwrap_or(0);
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
        let _ = fs::remove_file(&tmp_path);
        let _ = tx.send(UpdateEvent::Error {
            message: "Update download failed".to_string(),
            is_startup: false,
        });
        return;
    }

    // Validate downloaded installer size and PE MZ header
    let valid_file = fs::metadata(&tmp_path)
        .ok()
        .filter(|m| m.is_file() && m.len() >= MIN_INSTALLER_BYTES)
        .and_then(|_| fs::File::open(&tmp_path).ok())
        .and_then(|mut f| {
            let mut magic = [0u8; 2];
            f.read_exact(&mut magic).ok()?;
            Some(&magic == b"MZ")
        })
        .unwrap_or(false);

    if !valid_file {
        let _ = fs::remove_file(&tmp_path);
        let _ = tx.send(UpdateEvent::Error {
            message: "Downloaded installer verification failed".to_string(),
            is_startup: false,
        });
        return;
    }

    let final_bytes = fs::metadata(&tmp_path)
        .map(|m| m.len())
        .unwrap_or(info.asset_size);
    let _ = tx.send(UpdateEvent::DownloadProgress {
        downloaded_bytes: final_bytes,
        total_bytes: final_bytes.max(info.asset_size),
    });

    // Launch detached installer after a brief delay so Rustify can exit cleanly first
    let launch_cmd = format!(
        "ping -n 2 127.0.0.1 >NUL & start \"\" \"{}\"",
        tmp_path.display()
    );
    let mut launcher = Command::new("cmd");
    launcher
        .args(["/C", &launch_cmd])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    launcher.creation_flags(CREATE_NO_WINDOW);

    if launcher.spawn().is_err() {
        let _ = tx.send(UpdateEvent::Error {
            message: "Failed to launch update installer".to_string(),
            is_startup: false,
        });
        return;
    }

    let _ = tx.send(UpdateEvent::ReadyToInstall {
        installed_size: info.asset_size,
    });
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
}
