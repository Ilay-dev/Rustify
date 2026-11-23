use crossbeam_channel::{unbounded, RecvTimeoutError, Sender};
use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_DISCORD_CLIENT_ID: &str = "1574490783363506296";

const RUSTIFY_LARGE_IMAGE_URL: &str =
    "https://raw.githubusercontent.com/Ilay-dev/Rustify/main/assets/rustify_256.png";
const RUSTIFY_DOWNLOAD_URL: &str = "https://ilay-dev.github.io/Rustify-Download/";
const RETRY_INTERVAL: Duration = Duration::from_secs(5);
const SEEK_THRESHOLD_SECS: f32 = 2.5;

fn effective_client_id(custom: &str) -> &str {
    let t = custom.trim();
    if t.is_empty() {
        DEFAULT_DISCORD_CLIENT_ID
    } else {
        t
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RpcStatus {
    Disabled = 0,
    WaitingForDiscord = 1,
    Connected = 2,
}

impl RpcStatus {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => RpcStatus::Disabled,
            1 => RpcStatus::WaitingForDiscord,
            2 => RpcStatus::Connected,
            _ => RpcStatus::Disabled,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RpcTrackInfo {
    pub title: String,
    pub artist: String,
    pub playlist: String,
    pub position_sec: f32,
    pub duration_sec: f32,
    pub is_playing: bool,
}

enum RpcCommand {
    SetConfig { enabled: bool, client_id: String },
    UpdateTrack(Option<RpcTrackInfo>),
    Shutdown,
}

pub struct DiscordRpcManager {
    tx: Sender<RpcCommand>,
    status: Arc<AtomicU8>,
    last_sent: Option<RpcTrackInfo>,
    last_sent_at: Instant,
}

impl DiscordRpcManager {
    pub fn new(enabled: bool, client_id: String) -> Self {
        let trimmed_id = client_id.trim().to_string();
        let initial_status = if !enabled {
            RpcStatus::Disabled
        } else {
            RpcStatus::WaitingForDiscord
        };

        let status = Arc::new(AtomicU8::new(initial_status as u8));
        let status_bg = status.clone();
        let (tx, rx) = unbounded::<RpcCommand>();

        let _ = thread::Builder::new()
            .name("rustify-discord-rpc".to_string())
            .spawn(move || {
                rpc_worker_loop(enabled, trimmed_id, rx, status_bg);
            });

        Self {
            tx,
            status,
            last_sent: None,
            last_sent_at: Instant::now(),
        }
    }

    pub fn configure(&mut self, enabled: bool, client_id: String) {
        let _ = self.tx.send(RpcCommand::SetConfig {
            enabled,
            client_id: client_id.trim().to_string(),
        });
    }

    pub fn status(&self) -> RpcStatus {
        RpcStatus::from_u8(self.status.load(Ordering::Relaxed))
    }

    /// Rate-limited track state sync: only dispatches IPC updates on track change,
    /// play/pause transition, or manual seek > 2.5 seconds.
    pub fn update_track(&mut self, track: Option<RpcTrackInfo>) {
        let should_send = match (&self.last_sent, &track) {
            (None, None) => false,
            (Some(_), None) | (None, Some(_)) => true,
            (Some(prev), Some(curr)) => {
                if prev.title != curr.title
                    || prev.artist != curr.artist
                    || prev.playlist != curr.playlist
                    || prev.is_playing != curr.is_playing
                {
                    true
                } else if curr.is_playing {
                    let expected_pos = prev.position_sec + self.last_sent_at.elapsed().as_secs_f32();
                    (curr.position_sec - expected_pos).abs() > SEEK_THRESHOLD_SECS
                } else {
                    false
                }
            }
        };

        if should_send {
            self.last_sent = track.clone();
            self.last_sent_at = Instant::now();
            let _ = self.tx.send(RpcCommand::UpdateTrack(track));
        }
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(RpcCommand::Shutdown);
    }
}

impl Drop for DiscordRpcManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn set_status(atom: &AtomicU8, status: RpcStatus) {
    atom.store(status as u8, Ordering::Relaxed);
}

fn clamp_discord_field(input: &str, fallback: &str) -> String {
    let trimmed = input.trim();
    let base = if trimmed.is_empty() { fallback } else { trimmed };
    let mut out = String::new();
    for (idx, ch) in base.chars().enumerate() {
        if idx >= 125 {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    if out.chars().count() < 2 {
        out.push(' ');
    }
    out
}

fn disconnect_client(client: &mut Option<DiscordIpcClient>, activity_set: &mut bool) {
    if let Some(mut c) = client.take() {
        if *activity_set && c.clear_activity().is_ok() {
            let _ = c.recv();
        }
        let _ = c.close();
    }
    *activity_set = false;
}

fn push_activity(
    client: &mut DiscordIpcClient,
    track: Option<&RpcTrackInfo>,
    track_updated_at: Instant,
    activity_set: &mut bool,
) -> Result<(), ()> {
    match track {
        Some(track) if track.is_playing => {
            let details = clamp_discord_field(&track.title, "Rustify");
            let state = clamp_discord_field(&track.artist, "Local Music");
            let large_text = if track.playlist.trim().is_empty() {
                "Rustify".to_string()
            } else {
                clamp_discord_field(&format!("Rustify · {}", track.playlist.trim()), "Rustify")
            };

            let live_pos = (track.position_sec + track_updated_at.elapsed().as_secs_f32()).max(0.0);
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let start_ms = now_ms.saturating_sub((live_pos * 1000.0).round() as i64);

            let mut timestamps = activity::Timestamps::new().start(start_ms);
            if track.duration_sec > 1.0 {
                let end_ms = start_ms.saturating_add((track.duration_sec * 1000.0).round() as i64);
                timestamps = timestamps.end(end_ms);
            }

            let assets = activity::Assets::new()
                .large_image(RUSTIFY_LARGE_IMAGE_URL)
                .large_text(&large_text);

            let payload = activity::Activity::new()
                .activity_type(activity::ActivityType::Listening)
                .status_display_type(activity::StatusDisplayType::Name)
                .details(&details)
                .state(&state)
                .timestamps(timestamps)
                .assets(assets)
                .buttons(vec![activity::Button::new("Get Rustify", RUSTIFY_DOWNLOAD_URL)]);

            client.set_activity(payload).map_err(|_| ())?;
            let (_op, resp) = client.recv().map_err(|_| ())?;
            if resp.get("evt").and_then(|v| v.as_str()) == Some("ERROR") {
                return Err(());
            }
            *activity_set = true;
            Ok(())
        }
        _ => {
            if *activity_set {
                client.clear_activity().map_err(|_| ())?;
                let _ = client.recv();
                *activity_set = false;
            }
            Ok(())
        }
    }
}

fn try_connect(
    client_id: &str,
    client: &mut Option<DiscordIpcClient>,
    current_track: Option<&RpcTrackInfo>,
    track_updated_at: Instant,
    activity_set: &mut bool,
    status: &AtomicU8,
    last_attempt: &mut Option<Instant>,
) {
    *last_attempt = Some(Instant::now());
    let mut ipc = DiscordIpcClient::new(effective_client_id(client_id));
    if ipc.connect().is_ok() {
        *activity_set = false;
        if push_activity(&mut ipc, current_track, track_updated_at, activity_set).is_ok() {
            *client = Some(ipc);
            set_status(status, RpcStatus::Connected);
            return;
        }
        let _ = ipc.close();
    }
    *client = None;
    *activity_set = false;
    set_status(status, RpcStatus::WaitingForDiscord);
}

fn rpc_worker_loop(
    mut enabled: bool,
    mut client_id: String,
    rx: crossbeam_channel::Receiver<RpcCommand>,
    status: Arc<AtomicU8>,
) {
    let mut client: Option<DiscordIpcClient> = None;
    let mut current_track: Option<RpcTrackInfo> = None;
    let mut track_updated_at = Instant::now();
    let mut activity_set = false;
    let mut last_attempt: Option<Instant> = None;

    if enabled {
        try_connect(
            &client_id,
            &mut client,
            current_track.as_ref(),
            track_updated_at,
            &mut activity_set,
            &status,
            &mut last_attempt,
        );
    }

    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(RpcCommand::SetConfig {
                enabled: new_enabled,
                client_id: new_id,
            }) => {
                let id_changed = effective_client_id(&client_id) != effective_client_id(&new_id);
                let was_enabled = enabled;
                enabled = new_enabled;
                client_id = new_id;

                if !enabled {
                    disconnect_client(&mut client, &mut activity_set);
                    last_attempt = None;
                    set_status(&status, RpcStatus::Disabled);
                    continue;
                }

                if id_changed || !was_enabled || client.is_none() {
                    disconnect_client(&mut client, &mut activity_set);
                    try_connect(
                        &client_id,
                        &mut client,
                        current_track.as_ref(),
                        track_updated_at,
                        &mut activity_set,
                        &status,
                        &mut last_attempt,
                    );
                }
            }
            Ok(RpcCommand::UpdateTrack(track)) => {
                current_track = track;
                track_updated_at = Instant::now();
                if !enabled {
                    continue;
                }
                if let Some(ref mut ipc) = client {
                    if push_activity(ipc, current_track.as_ref(), track_updated_at, &mut activity_set)
                        .is_err()
                    {
                        disconnect_client(&mut client, &mut activity_set);
                        last_attempt = Some(Instant::now());
                        set_status(&status, RpcStatus::WaitingForDiscord);
                    }
                }
            }
            Ok(RpcCommand::Shutdown) => {
                disconnect_client(&mut client, &mut activity_set);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                if enabled && client.is_none() {
                    let due = last_attempt
                        .map(|t| t.elapsed() >= RETRY_INTERVAL)
                        .unwrap_or(true);
                    if due {
                        try_connect(
                            &client_id,
                            &mut client,
                            current_track.as_ref(),
                            track_updated_at,
                            &mut activity_set,
                            &status,
                            &mut last_attempt,
                        );
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                disconnect_client(&mut client, &mut activity_set);
                break;
            }
        }
    }
}
