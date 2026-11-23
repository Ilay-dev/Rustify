pub mod icons;
pub mod player_bar;
pub mod playlist_import_modal;
pub mod playlist_view;
pub mod settings_modal;
pub mod setup;
pub mod shader;
pub mod sidebar;
pub mod theme;
pub mod visualizer;

use crate::audio::bpm::BpmManager;
use crate::audio::dj::build_dj_sorted_indices;
use crate::audio::engine::AudioEngine;
use crate::audio::{AudioCommand, AudioTrackSpec, DjSettings, EngineSnapshot};
use crate::config::{AppConfig, PlaybackMode, RepeatMode};
use crate::covers::{set_custom_cover, CoverManager};
use crate::discord_rpc::{DiscordRpcManager, RpcTrackInfo};
use crate::i18n::tr;
use crate::library::{
    create_playlist, delete_track, import_audio_files, import_pc_folder_as_playlist, rename_track,
    reveal_in_explorer, scan_library, Playlist, Track,
};
use crate::lyrics::{show_lyrics_panel, LyricsAction, LyricsManager};
use crate::media_controls::SystemMediaController;
use crate::ui::icons::Icon;
use crate::ui::player_bar::{show_player_bar, PlayerAction};
use crate::ui::playlist_import_modal::{
    show_playlist_import_modal, PlaylistImportAction, PlaylistImportModalState,
};
use crate::ui::playlist_view::{show_playlist_view, PlaylistAction, PlaylistViewInfo};
use crate::ui::settings_modal::{show_settings_modal, SettingsAction};
use crate::ui::setup::{show_setup_wizard, SetupState};
use crate::ui::shader::LiquidBackdrop;
use crate::ui::sidebar::{show_sidebar, SidebarAction, ViewSelection};
use crate::ui::theme::{
    draw_liquid_glass_panel, font_regular, font_semibold, frosted_cta_button, glass_icon_button,
    glass_pill_button, install_fonts_and_style, GlassLevel, DANGER_GLASS, GLASS_PILL_IDLE,
    RIM_SUBTLE, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use crate::ui::visualizer::{AudioVisualizerState, VisualizerConfig};
use egui::{
    Align2, Color32, ColorImage, Context, CornerRadius, Pos2, Rect, ResizeDirection, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Vec2, ViewportCommand,
};
use rand::seq::SliceRandom;
use souvlaki::MediaControlEvent;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LOGO_PNG: &[u8] = include_bytes!("../../assets/rustify_256.png");

struct RenameDialog {
    path: PathBuf,
    title: String,
    artist: String,
    error: Option<String>,
}

struct DeleteDialog {
    path: PathBuf,
    title: String,
}

struct Toast {
    message: String,
    created: Instant,
}

pub struct RustifyApp {
    config: AppConfig,
    setup_state: SetupState,
    playlists: Vec<Playlist>,
    selection: ViewSelection,
    search_query: String,
    focus_search: bool,

    // Playback & Queue state
    engine: AudioEngine,
    snap: EngineSnapshot,
    bpm: BpmManager,
    covers: CoverManager,
    lyrics_mgr: LyricsManager,
    media_controls: SystemMediaController,
    discord_rpc: DiscordRpcManager,

    play_queue: Vec<PathBuf>,
    queue_pos: usize,
    user_queue: VecDeque<PathBuf>,
    next_came_from_user_queue: bool,
    last_ended_counter: u64,
    last_transition_counter: u64,
    last_reported_audio_error: Option<String>,

    // UI state
    backdrop: Option<LiquidBackdrop>,
    logo_tex: Option<TextureHandle>,
    start_instant: Instant,
    creating_playlist: bool,
    new_playlist_name: String,
    queue_open: bool,
    lyrics_open: bool,
    settings_open: bool,
    mini_mode: bool,
    visualizer: AudioVisualizerState,
    import_modal: PlaylistImportModalState,
    rename_dialog: Option<RenameDialog>,
    delete_dialog: Option<DeleteDialog>,
    toast: Option<Toast>,
    capture_path: Option<PathBuf>,
    capture_frames: u32,
}

impl RustifyApp {
    pub fn new(cc: &eframe::CreationContext<'_>, hwnd: Option<*mut c_void>) -> Self {
        install_fonts_and_style(&cc.egui_ctx);

        let config = AppConfig::load();
        let setup_state = SetupState::new(config.music_dir.as_ref());

        let dj_settings = DjSettings {
            crossfade_secs: config.dj_crossfade_secs,
            tempo_sync: config.dj_tempo_sync,
            bass_swap: config.dj_bass_swap,
            trim_silence: config.dj_trim_silence,
            normal_short_fade: config.normal_short_fade,
        };
        let eff_vol = if config.muted { 0.0 } else { config.volume };
        let engine = AudioEngine::new(eff_vol, dj_settings);
        engine.send(AudioCommand::SetEq(config.eq_bands_db));
        engine.send(AudioCommand::SetLoudnessNorm(config.loudness_normalization));
        let bpm = BpmManager::new();
        let media_controls = SystemMediaController::new(hwnd);
        let discord_rpc = DiscordRpcManager::new(
            config.discord_rpc_enabled,
            config.discord_client_id.clone(),
        );

        let backdrop = cc.gl.as_ref().map(|gl| LiquidBackdrop::new(gl));
        let logo_tex = load_logo_texture(&cc.egui_ctx);

        let mut app = Self {
            config,
            setup_state,
            playlists: Vec::new(),
            selection: ViewSelection::Playlist(0),
            search_query: String::new(),
            focus_search: false,
            engine,
            snap: EngineSnapshot::default(),
            bpm,
            covers: CoverManager::new(),
            lyrics_mgr: LyricsManager::new(),
            media_controls,
            discord_rpc,
            play_queue: Vec::new(),
            queue_pos: 0,
            user_queue: VecDeque::new(),
            next_came_from_user_queue: false,
            last_ended_counter: 0,
            last_transition_counter: 0,
            last_reported_audio_error: None,
            backdrop,
            logo_tex,
            start_instant: Instant::now(),
            creating_playlist: false,
            new_playlist_name: String::new(),
            queue_open: false,
            lyrics_open: false,
            settings_open: false,
            mini_mode: false,
            visualizer: AudioVisualizerState::new(VisualizerConfig::default()),
            import_modal: PlaylistImportModalState::default(),
            rename_dialog: None,
            delete_dialog: None,
            toast: None,
            capture_path: std::env::var_os("RUSTIFY_CAPTURE_PNG").map(PathBuf::from),
            capture_frames: 0,
        };

        if let Ok(mode) = std::env::var("RUSTIFY_CAPTURE_VIEW") {
            match mode.as_str() {
                "setup" => {
                    app.config.setup_completed = false;
                }
                "main" => {
                    app.config.setup_completed = true;
                    app.config.playback_mode = PlaybackMode::DjMix;
                }
                "settings" => {
                    app.config.setup_completed = true;
                    app.settings_open = true;
                }
                "import" => {
                    app.config.setup_completed = true;
                    app.import_modal.open_from_sidebar();
                }
                "lyrics" => {
                    app.config.setup_completed = true;
                    app.lyrics_open = true;
                }
                "visualizer" => {
                    app.config.setup_completed = true;
                    app.visualizer.open = true;
                    app.visualizer.config.preset = crate::ui::visualizer::VisualizerPreset::Custom;
                }
                "mini" => {
                    app.config.setup_completed = true;
                    app.mini_mode = true;
                    cc.egui_ctx
                        .send_viewport_cmd(ViewportCommand::MinInnerSize(Vec2::new(340.0, 86.0)));
                    cc.egui_ctx
                        .send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(380.0, 92.0)));
                }
                _ => {}
            }
        }

        if app.config.setup_completed {
            app.reload_library();
            match std::env::var("RUSTIFY_CAPTURE_VIEW").as_deref() {
                Ok("main") => {
                    let (_, _, _, tracks) = app.collect_view_tracks();
                    let disp = app.build_display_indices(&tracks);
                    let visible_tracks: Vec<Track> = disp
                        .into_iter()
                        .take(12)
                        .filter_map(|i| tracks.get(i).cloned())
                        .collect();
                    app.covers.fetch_missing_for_tracks(&visible_tracks);
                    if let Some(first) = visible_tracks.get(1).or_else(|| visible_tracks.first()) {
                        cc.egui_ctx.data_mut(|d| {
                            d.insert_temp(
                                egui::Id::new("rustify_open_track_menu"),
                                first.path.clone(),
                            );
                        });
                    }
                }
                Ok("visualizer") => {
                    let (_, _, _, tracks) = app.collect_view_tracks();
                    let disp = app.build_display_indices(&tracks);
                    let visible_tracks: Vec<Track> = disp
                        .into_iter()
                        .take(8)
                        .filter_map(|i| tracks.get(i).cloned())
                        .collect();
                    app.covers.fetch_missing_for_tracks(&visible_tracks);
                }
                _ => {}
            }
        }
        app
    }

    fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast {
            message: msg.into(),
            created: Instant::now(),
        });
    }

    fn reload_library(&mut self) {
        let Some(ref dir) = self.config.music_dir else {
            self.playlists.clear();
            return;
        };
        self.playlists = scan_library(dir);

        for pl in &mut self.playlists {
            for t in &mut pl.tracks {
                if let Some(cached) = self.bpm.get_cached(&t.path) {
                    t.analysis = Some(cached);
                } else if self.config.auto_analyze_bpm {
                    self.bpm.enqueue_if_needed(&t.path);
                }
            }
        }

        if let Some(ref last_name) = self.config.last_playlist {
            if let Some(idx) = self.playlists.iter().position(|p| &p.name == last_name) {
                self.selection = ViewSelection::Playlist(idx);
                return;
            }
        }
        let ViewSelection::Playlist(idx) = self.selection;
        if idx >= self.playlists.len() {
            self.selection = ViewSelection::Playlist(0);
        }
    }

    fn find_track(&self, path: &Path) -> Option<&Track> {
        for pl in &self.playlists {
            if let Some(t) = pl.tracks.iter().find(|t| t.path == path) {
                return Some(t);
            }
        }
        None
    }

    fn make_track_spec(&self, path: &Path) -> Option<AudioTrackSpec> {
        let t = self.find_track(path)?;
        Some(AudioTrackSpec {
            path: t.path.clone(),
            title: t.title.clone(),
            artist: t.artist.clone(),
            analysis: t.analysis.clone(),
        })
    }

    fn collect_view_tracks(&self) -> (String, bool, Option<PathBuf>, Vec<Track>) {
        let ViewSelection::Playlist(idx) = self.selection;
        if let Some(pl) = self.playlists.get(idx) {
            (
                pl.name.clone(),
                true,
                Some(pl.dir.clone()),
                pl.tracks.clone(),
            )
        } else {
            ("Rustify".to_string(), false, None, Vec::new())
        }
    }

    fn build_display_indices(&self, tracks: &[Track]) -> Vec<usize> {
        let base_order: Vec<usize> = if self.config.playback_mode == PlaybackMode::DjMix {
            let analyses: Vec<_> = tracks.iter().map(|t| t.analysis.clone()).collect();
            build_dj_sorted_indices(&analyses)
        } else {
            (0..tracks.len()).collect()
        };

        let q = self.search_query.trim().to_lowercase();
        if q.is_empty() {
            return base_order;
        }
        base_order
            .into_iter()
            .filter(|&idx| {
                let t = &tracks[idx];
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.bpm()
                        .map(|b| format!("{:.0}", b).contains(&q))
                        .unwrap_or(false)
            })
            .collect()
    }

    fn build_queue_from_tracks(&mut self, tracks: &[Track], anchor_path: Option<&Path>) {
        if tracks.is_empty() {
            self.play_queue.clear();
            self.queue_pos = 0;
            return;
        }

        match self.config.playback_mode {
            PlaybackMode::Normal => {
                self.play_queue = tracks.iter().map(|t| t.path.clone()).collect();
                self.queue_pos = anchor_path
                    .and_then(|ap| self.play_queue.iter().position(|p| p == ap))
                    .unwrap_or(0);
            }
            PlaybackMode::Shuffle => {
                let mut paths: Vec<PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
                let mut rng = rand::rng();
                paths.shuffle(&mut rng);
                if let Some(ap) = anchor_path {
                    if let Some(pos) = paths.iter().position(|p| p == ap) {
                        paths.swap(0, pos);
                    }
                }
                self.play_queue = paths;
                self.queue_pos = 0;
            }
            PlaybackMode::DjMix => {
                let analyses: Vec<_> = tracks.iter().map(|t| t.analysis.clone()).collect();
                let sorted = build_dj_sorted_indices(&analyses);
                self.play_queue = sorted.into_iter().map(|i| tracks[i].path.clone()).collect();
                self.queue_pos = anchor_path
                    .and_then(|ap| self.play_queue.iter().position(|p| p == ap))
                    .unwrap_or(0);
            }
        }
    }

    fn rebuild_active_queue_for_mode(&mut self) {
        if self.play_queue.is_empty() {
            return;
        }
        let current_path = self
            .snap
            .current_path
            .clone()
            .or_else(|| self.play_queue.get(self.queue_pos).cloned());

        let mut tracks = Vec::with_capacity(self.play_queue.len());
        for p in &self.play_queue {
            if let Some(t) = self.find_track(p) {
                tracks.push(t.clone());
            }
        }
        if tracks.is_empty() {
            return;
        }

        tracks.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        self.build_queue_from_tracks(&tracks, current_path.as_deref());
        self.sync_next_preload();
    }

    fn peek_next_path(&self) -> (Option<PathBuf>, bool) {
        if self.config.repeat_mode == RepeatMode::One {
            return (self.play_queue.get(self.queue_pos).cloned(), false);
        }
        if let Some(uq) = self.user_queue.front() {
            return (Some(uq.clone()), true);
        }
        if self.play_queue.is_empty() {
            return (None, false);
        }
        let next_idx = self.queue_pos + 1;
        if next_idx < self.play_queue.len() {
            (Some(self.play_queue[next_idx].clone()), false)
        } else if self.config.repeat_mode == RepeatMode::All && self.play_queue.len() > 1 {
            (Some(self.play_queue[0].clone()), false)
        } else {
            (None, false)
        }
    }

    fn sync_next_preload(&mut self) {
        let (next_path, from_uq) = self.peek_next_path();
        self.next_came_from_user_queue = from_uq;
        let next_spec = next_path.as_deref().and_then(|p| self.make_track_spec(p));
        self.engine.send(AudioCommand::UpdateNext {
            next: next_spec,
            mode: self.config.playback_mode,
        });
    }

    fn play_track_from_view(&mut self, tracks: &[Track], track_idx: usize) {
        let Some(track) = tracks.get(track_idx) else {
            return;
        };
        let path = track.path.clone();

        if self.snap.current_path.as_ref() == Some(&path) {
            self.engine.send(AudioCommand::TogglePlay);
            return;
        }

        self.build_queue_from_tracks(tracks, Some(&path));
        let Some(spec) = self.make_track_spec(&path) else {
            return;
        };
        let (next_path, from_uq) = self.peek_next_path();
        self.next_came_from_user_queue = from_uq;
        let next_spec = next_path.as_deref().and_then(|p| self.make_track_spec(p));

        if self.config.playback_mode == PlaybackMode::DjMix && self.snap.is_playing {
            self.engine.send(AudioCommand::CrossfadeTo {
                track: spec,
                next: next_spec,
                mode: self.config.playback_mode,
            });
        } else {
            self.engine.send(AudioCommand::PlayNow {
                track: spec,
                next: next_spec,
                mode: self.config.playback_mode,
            });
        }
    }

    fn play_next(&mut self, allow_dj_fade: bool) {
        if !allow_dj_fade && self.config.repeat_mode == RepeatMode::One {
            if let Some(path) = self.play_queue.get(self.queue_pos).cloned() {
                if let Some(spec) = self.make_track_spec(&path) {
                    let (next_path, from_uq) = self.peek_next_path();
                    self.next_came_from_user_queue = from_uq;
                    let next_spec = next_path.as_deref().and_then(|p| self.make_track_spec(p));
                    self.engine.send(AudioCommand::PlayNow {
                        track: spec,
                        next: next_spec,
                        mode: self.config.playback_mode,
                    });
                }
                return;
            }
        }

        if let Some(uq_path) = self.user_queue.pop_front() {
            if let Some(spec) = self.make_track_spec(&uq_path) {
                let (next_path, from_uq) = self.peek_next_path();
                self.next_came_from_user_queue = from_uq;
                let next_spec = next_path.as_deref().and_then(|p| self.make_track_spec(p));
                if allow_dj_fade
                    && self.config.playback_mode == PlaybackMode::DjMix
                    && self.snap.is_playing
                {
                    self.engine.send(AudioCommand::CrossfadeTo {
                        track: spec,
                        next: next_spec,
                        mode: self.config.playback_mode,
                    });
                } else {
                    self.engine.send(AudioCommand::PlayNow {
                        track: spec,
                        next: next_spec,
                        mode: self.config.playback_mode,
                    });
                }
            }
            return;
        }

        if self.play_queue.is_empty() {
            return;
        }

        let next_idx = if self.queue_pos + 1 < self.play_queue.len() {
            Some(self.queue_pos + 1)
        } else if self.config.repeat_mode == RepeatMode::All {
            Some(0)
        } else {
            None
        };

        let Some(idx) = next_idx else {
            self.engine.send(AudioCommand::Stop);
            return;
        };

        self.queue_pos = idx;
        let path = self.play_queue[idx].clone();
        if let Some(spec) = self.make_track_spec(&path) {
            let (next_path, from_uq) = self.peek_next_path();
            self.next_came_from_user_queue = from_uq;
            let next_spec = next_path.as_deref().and_then(|p| self.make_track_spec(p));
            if allow_dj_fade
                && self.config.playback_mode == PlaybackMode::DjMix
                && self.snap.is_playing
            {
                self.engine.send(AudioCommand::CrossfadeTo {
                    track: spec,
                    next: next_spec,
                    mode: self.config.playback_mode,
                });
            } else {
                self.engine.send(AudioCommand::PlayNow {
                    track: spec,
                    next: next_spec,
                    mode: self.config.playback_mode,
                });
            }
        }
    }

    fn play_prev(&mut self) {
        if self.snap.position_sec > 3.2 && self.snap.current_path.is_some() {
            self.engine.send(AudioCommand::Seek(0.0));
            return;
        }
        if self.play_queue.is_empty() {
            return;
        }
        if self.queue_pos > 0 {
            self.queue_pos -= 1;
        } else if self.config.repeat_mode == RepeatMode::All {
            self.queue_pos = self.play_queue.len().saturating_sub(1);
        } else {
            self.engine.send(AudioCommand::Seek(0.0));
            return;
        }

        let path = self.play_queue[self.queue_pos].clone();
        if let Some(spec) = self.make_track_spec(&path) {
            let (next_path, from_uq) = self.peek_next_path();
            self.next_came_from_user_queue = from_uq;
            let next_spec = next_path.as_deref().and_then(|p| self.make_track_spec(p));
            self.engine.send(AudioCommand::PlayNow {
                track: spec,
                next: next_spec,
                mode: self.config.playback_mode,
            });
        }
    }

    fn poll_background_events(&mut self) {
        self.covers.poll();
        self.lyrics_mgr.poll();
        let updates = self.bpm.poll_updates();
        if !updates.is_empty() {
            for upd in updates {
                for pl in &mut self.playlists {
                    for t in &mut pl.tracks {
                        if t.path == upd.path {
                            t.analysis = Some(upd.analysis.clone());
                            if t.duration_sec.is_none() {
                                t.duration_sec = Some(upd.analysis.duration_sec);
                            }
                        }
                    }
                }
                self.engine.send(AudioCommand::UpdateCurrentAnalysis {
                    path: upd.path,
                    analysis: upd.analysis,
                });
            }
            // Only re-sort active queue once the analysis batch completes and no mix is mid-flight
            if self.config.playback_mode == PlaybackMode::DjMix
                && !self.bpm.is_analyzing()
                && !self.snap.mix.active
            {
                self.rebuild_active_queue_for_mode();
            }
        }

        self.snap = self.engine.snapshot();
        if self.snap.last_error != self.last_reported_audio_error {
            self.last_reported_audio_error = self.snap.last_error.clone();
            if let Some(ref err) = self.last_reported_audio_error {
                self.show_toast(err.clone());
            }
        }

        if self.snap.transitioned_to_next_counter > self.last_transition_counter {
            self.last_transition_counter = self.snap.transitioned_to_next_counter;
            if self.config.repeat_mode != RepeatMode::One {
                if self.next_came_from_user_queue {
                    self.user_queue.pop_front();
                } else if let Some(ref cp) = self.snap.current_path {
                    if let Some(pos) = self.play_queue.iter().position(|p| p == cp) {
                        self.queue_pos = pos;
                    }
                }
            }
            self.sync_next_preload();
        } else if self.snap.track_ended_counter > self.last_ended_counter {
            self.last_ended_counter = self.snap.track_ended_counter;
            self.play_next(false);
        }

        for ev in self.media_controls.poll_events() {
            match ev {
                MediaControlEvent::Play => {
                    if self.snap.current_path.is_some() {
                        self.engine.send(AudioCommand::SetPlaying(true));
                    } else {
                        let (_, _, _, tracks) = self.collect_view_tracks();
                        if !tracks.is_empty() {
                            self.play_track_from_view(&tracks, 0);
                        }
                    }
                }
                MediaControlEvent::Pause => self.engine.send(AudioCommand::SetPlaying(false)),
                MediaControlEvent::Toggle => {
                    if self.snap.current_path.is_some() {
                        self.engine.send(AudioCommand::TogglePlay);
                    } else {
                        let (_, _, _, tracks) = self.collect_view_tracks();
                        if !tracks.is_empty() {
                            self.play_track_from_view(&tracks, 0);
                        }
                    }
                }
                MediaControlEvent::Next => self.play_next(true),
                MediaControlEvent::Previous => self.play_prev(),
                MediaControlEvent::Stop => self.engine.send(AudioCommand::Stop),
                MediaControlEvent::SetPosition(pos) => {
                    self.engine.send(AudioCommand::Seek(pos.0.as_secs_f32()));
                }
                _ => {}
            }
        }

        let s = tr(self.config.language);
        let meta_owned: Option<(String, String, String, f32)> = self
            .snap
            .current_path
            .as_deref()
            .and_then(|p| self.find_track(p))
            .map(|t| {
                (
                    t.title.clone(),
                    t.display_artist(s.unknown_artist).to_string(),
                    t.playlist_name.clone(),
                    t.effective_duration(),
                )
            });
        let dur = if self.snap.duration_sec > 0.1 {
            self.snap.duration_sec
        } else {
            meta_owned.as_ref().map(|m| m.3).unwrap_or(0.0)
        };
        self.media_controls.update_state(
            meta_owned.as_ref().map(|m| m.0.as_str()),
            meta_owned.as_ref().map(|m| m.1.as_str()),
            meta_owned.as_ref().map(|m| m.2.as_str()),
            dur,
            self.snap.position_sec,
            self.snap.is_playing,
        );

        let rpc_info = meta_owned.map(|(title, artist, playlist, _)| RpcTrackInfo {
            title,
            artist,
            playlist,
            position_sec: self.snap.position_sec,
            duration_sec: dur,
            is_playing: self.snap.is_playing,
        });
        self.discord_rpc.update_track(rpc_info);
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &Context) {
        if !self.config.setup_completed {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.rename_dialog.is_some() {
                self.rename_dialog = None;
                return;
            }
            if self.delete_dialog.is_some() {
                self.delete_dialog = None;
                return;
            }
            if self.import_modal.open {
                self.import_modal.open = false;
                return;
            }
            if self.settings_open {
                self.settings_open = false;
                return;
            }
            if self.lyrics_open {
                self.lyrics_open = false;
                return;
            }
            if self.queue_open {
                self.queue_open = false;
                return;
            }
            if !self.search_query.is_empty() {
                self.search_query.clear();
                return;
            }
        }

        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F)) {
            self.focus_search = true;
            return;
        }

        if ctx.wants_keyboard_input() {
            return;
        }

        let (space, left, right, up, down, key_m, ctrl) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::M),
                i.modifiers.ctrl,
            )
        });

        if space {
            if self.snap.current_path.is_some() {
                self.engine.send(AudioCommand::TogglePlay);
            } else {
                let (_, _, _, tracks) = self.collect_view_tracks();
                if !tracks.is_empty() {
                    self.play_track_from_view(&tracks, 0);
                }
            }
        }
        if left {
            if ctrl {
                self.play_prev();
            } else if self.snap.duration_sec > 0.5 {
                let target = (self.snap.position_sec - 5.0).max(0.0);
                self.engine.send(AudioCommand::Seek(target));
            }
        }
        if right {
            if ctrl {
                self.play_next(true);
            } else if self.snap.duration_sec > 0.5 {
                let target = (self.snap.position_sec + 5.0).min(self.snap.duration_sec - 0.5);
                self.engine.send(AudioCommand::Seek(target));
            }
        }
        if up {
            self.config.muted = false;
            self.config.volume = (self.config.volume + 0.05).clamp(0.0, 1.0);
            self.engine.send(AudioCommand::SetVolume(self.config.volume));
            self.config.save();
        }
        if down {
            self.config.muted = false;
            self.config.volume = (self.config.volume - 0.05).clamp(0.0, 1.0);
            self.engine.send(AudioCommand::SetVolume(self.config.volume));
            self.config.save();
        }
        if key_m {
            self.config.muted = !self.config.muted;
            let v = if self.config.muted { 0.0 } else { self.config.volume };
            self.engine.send(AudioCommand::SetVolume(v));
            self.config.save();
        }
    }

    fn handle_file_drops(&mut self, ctx: &Context) -> bool {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            let ViewSelection::Playlist(idx) = self.selection;
            if let Some(pl) = self.playlists.get(idx) {
                let count = import_audio_files(&pl.dir, &dropped);
                if count > 0 {
                    self.reload_library();
                    let s = tr(self.config.language);
                    self.show_toast(format!("{count} {}", s.toast_imported_files));
                }
            }
        }
        hovering
    }

    fn set_mini_mode(&mut self, ctx: &Context, mini: bool) {
        self.mini_mode = mini;
        if mini {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(Vec2::new(340.0, 86.0)));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(380.0, 92.0)));
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
        } else {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(Vec2::new(920.0, 600.0)));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(1240.0, 780.0)));
        }
    }

    fn draw_mini_widget(&mut self, ui: &mut egui::Ui, screen: Rect) {
        let ctx = ui.ctx().clone();
        let s = tr(self.config.language);

        // Background drag area (excluding control buttons on the right)
        let drag_rect = Rect::from_min_max(
            screen.min,
            Pos2::new((screen.right() - 156.0).max(screen.left() + 60.0), screen.bottom()),
        );
        let drag_resp = ui.interact(drag_rect, ui.id().with("mini_drag"), Sense::click_and_drag());
        if drag_resp.double_clicked() {
            self.set_mini_mode(&ctx, false);
            return;
        } else if drag_resp.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        let cur_track = self
            .snap
            .current_path
            .as_deref()
            .and_then(|p| self.find_track(p))
            .cloned();

        let p = ui.painter();
        let text_left = screen.left() + 18.0;
        let text_right = (screen.right() - 158.0).max(text_left + 40.0);

        if self.snap.mix.active {
            let mix_title = format!("{}  ⇄  {}", self.snap.mix.outgoing_title, self.snap.mix.incoming_title);
            crate::ui::theme::draw_truncated_text(
                p,
                Rect::from_min_max(
                    Pos2::new(text_left, screen.center().y - 19.0),
                    Pos2::new(text_right, screen.center().y + 1.0),
                ),
                &mix_title,
                font_semibold(14.0),
                TEXT_PRIMARY,
                Align2::LEFT_CENTER,
            );
            crate::ui::theme::draw_truncated_text(
                p,
                Rect::from_min_max(
                    Pos2::new(text_left, screen.center().y + 2.0),
                    Pos2::new(text_right, screen.center().y + 19.0),
                ),
                &format!("{} {:.0}%", s.dj_mixing, self.snap.mix.progress * 100.0),
                font_regular(12.0),
                TEXT_SECONDARY,
                Align2::LEFT_CENTER,
            );
        } else if let Some(ref track) = cur_track {
            crate::ui::theme::draw_truncated_text(
                p,
                Rect::from_min_max(
                    Pos2::new(text_left, screen.center().y - 19.0),
                    Pos2::new(text_right, screen.center().y + 1.0),
                ),
                &track.title,
                font_semibold(14.0),
                TEXT_PRIMARY,
                Align2::LEFT_CENTER,
            );
            crate::ui::theme::draw_truncated_text(
                p,
                Rect::from_min_max(
                    Pos2::new(text_left, screen.center().y + 2.0),
                    Pos2::new(text_right, screen.center().y + 19.0),
                ),
                track.display_artist(s.unknown_artist),
                font_regular(12.0),
                TEXT_SECONDARY,
                Align2::LEFT_CENTER,
            );
        } else {
            crate::ui::theme::draw_truncated_text(
                p,
                Rect::from_min_max(
                    Pos2::new(text_left, screen.center().y - 18.0),
                    Pos2::new(text_right, screen.center().y + 1.0),
                ),
                s.nothing_playing,
                font_semibold(13.5),
                TEXT_SECONDARY,
                Align2::LEFT_CENTER,
            );
            crate::ui::theme::draw_truncated_text(
                p,
                Rect::from_min_max(
                    Pos2::new(text_left, screen.center().y + 2.0),
                    Pos2::new(text_right, screen.center().y + 18.0),
                ),
                s.select_track_hint,
                font_regular(11.5),
                TEXT_MUTED,
                Align2::LEFT_CENTER,
            );
        }

        // Right-side controls: [ Prev ] [ Play/Pause ] [ Next ]  [ Expand ]
        let ctrl_rect = Rect::from_min_max(
            Pos2::new(screen.right() - 152.0, screen.top() + 8.0),
            Pos2::new(screen.right() - 12.0, screen.bottom() - 8.0),
        );
        let mut c_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(ctrl_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        c_ui.spacing_mut().item_spacing.x = 6.0;

        if glass_icon_button(&mut c_ui, Icon::Prev, 28.0, 13.0, false, false).clicked() {
            self.play_prev();
        }
        let play_ic = if self.snap.is_playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        if glass_icon_button(&mut c_ui, play_ic, 34.0, 14.0, true, true).clicked() {
            if self.snap.current_path.is_some() {
                self.engine.send(AudioCommand::TogglePlay);
            } else {
                let (_, _, _, tracks) = self.collect_view_tracks();
                if !tracks.is_empty() {
                    self.play_track_from_view(&tracks, 0);
                }
            }
        }
        if glass_icon_button(&mut c_ui, Icon::Next, 28.0, 13.0, false, false).clicked() {
            self.play_next(true);
        }
        c_ui.add_space(4.0);
        if glass_icon_button(&mut c_ui, Icon::Expand, 28.0, 12.0, false, false)
            .on_hover_text("Vollansicht / Expand")
            .clicked()
        {
            self.set_mini_mode(&ctx, false);
        }

        // Subtle 2px progress line along bottom edge
        let dur = if self.snap.duration_sec > 0.1 {
            self.snap.duration_sec
        } else {
            cur_track.as_ref().map(|t| t.effective_duration()).unwrap_or(0.0)
        };
        if dur > 0.5 {
            let frac = (self.snap.position_sec / dur).clamp(0.0, 1.0);
            let bar_bg = Rect::from_min_max(
                Pos2::new(screen.left() + 12.0, screen.bottom() - 3.5),
                Pos2::new(screen.right() - 12.0, screen.bottom() - 1.5),
            );
            ui.painter().rect_filled(
                bar_bg,
                CornerRadius::same(1),
                Color32::from_rgba_unmultiplied(255, 255, 255, 20),
            );
            let bar_fill = Rect::from_min_max(
                bar_bg.min,
                Pos2::new(bar_bg.left() + bar_bg.width() * frac, bar_bg.bottom()),
            );
            ui.painter()
                .rect_filled(bar_fill, CornerRadius::same(1), Color32::WHITE);
        }
    }

    fn draw_titlebar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let ctx = ui.ctx().clone();
        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));

        let drag_rect = Rect::from_min_max(rect.min, Pos2::new(rect.right() - 154.0, rect.bottom()));
        let drag_resp =
            ui.interact(drag_rect, ui.id().with("titlebar_drag"), Sense::click_and_drag());
        if drag_resp.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized));
        } else if drag_resp.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        let p = ui.painter();

        let logo_rect = Rect::from_center_size(
            Pos2::new(rect.left() + 24.0, rect.center().y),
            Vec2::splat(20.0),
        );
        if let Some(ref tex) = self.logo_tex {
            p.image(
                tex.id(),
                logo_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        p.text(
            Pos2::new(rect.left() + 40.0, rect.center().y),
            Align2::LEFT_CENTER,
            "Rustify",
            font_semibold(13.0),
            TEXT_PRIMARY,
        );

        let ctrl_rect = Rect::from_min_max(
            Pos2::new(rect.right() - 148.0, rect.top()),
            Pos2::new(rect.right() - 8.0, rect.bottom()),
        );
        let mut c_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(ctrl_rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        c_ui.spacing_mut().item_spacing.x = 6.0;

        if glass_icon_button(&mut c_ui, Icon::Close, 26.0, 10.5, false, false).clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        let max_icon = if is_maximized {
            Icon::Restore
        } else {
            Icon::Maximize
        };
        if glass_icon_button(&mut c_ui, max_icon, 26.0, 10.5, false, false).clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized));
        }
        if glass_icon_button(&mut c_ui, Icon::Minimize, 26.0, 10.5, false, false).clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        if self.config.setup_completed
            && glass_icon_button(&mut c_ui, Icon::MiniPlayer, 26.0, 11.5, false, false)
                .on_hover_text("Mini-Widget")
                .clicked()
        {
            self.set_mini_mode(&ctx, true);
        }
    }

    fn handle_window_resize_edges(&self, ctx: &Context, screen: Rect) {
        if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let m = 6.0;
        let left = pos.x <= screen.left() + m;
        let right = pos.x >= screen.right() - m;
        let top = pos.y <= screen.top() + m;
        let bottom = pos.y >= screen.bottom() - m;

        let dir = match (left, right, top, bottom) {
            (true, _, true, _) => Some(ResizeDirection::NorthWest),
            (_, true, true, _) => Some(ResizeDirection::NorthEast),
            (true, _, _, true) => Some(ResizeDirection::SouthWest),
            (_, true, _, true) => Some(ResizeDirection::SouthEast),
            (true, _, _, _) => Some(ResizeDirection::West),
            (_, true, _, _) => Some(ResizeDirection::East),
            (_, _, true, _) => Some(ResizeDirection::North),
            (_, _, _, true) => Some(ResizeDirection::South),
            _ => None,
        };

        if let Some(d) = dir {
            let cursor = match d {
                ResizeDirection::North | ResizeDirection::South => egui::CursorIcon::ResizeVertical,
                ResizeDirection::East | ResizeDirection::West => egui::CursorIcon::ResizeHorizontal,
                ResizeDirection::NorthWest | ResizeDirection::SouthEast => {
                    egui::CursorIcon::ResizeNwSe
                }
                ResizeDirection::NorthEast | ResizeDirection::SouthWest => {
                    egui::CursorIcon::ResizeNeSw
                }
            };
            ctx.set_cursor_icon(cursor);
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(d));
            }
        }
    }

    fn show_rename_dialog(&mut self, ui: &mut egui::Ui, screen: Rect) {
        let Some(ref mut dlg) = self.rename_dialog else {
            return;
        };
        let s = tr(self.config.language);
        ui.painter().rect_filled(
            screen,
            CornerRadius::ZERO,
            Color32::from_rgba_unmultiplied(0, 0, 0, 175),
        );
        let card = Rect::from_center_size(screen.center(), Vec2::new(420.0, 260.0));
        draw_liquid_glass_panel(ui.painter(), card, 18, GlassLevel::Modal);

        let inner = card.shrink2(Vec2::new(24.0, 20.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        c.label(
            egui::RichText::new(s.rename_title)
                .font(font_semibold(17.0))
                .color(TEXT_PRIMARY),
        );
        c.add_space(14.0);

        c.label(
            egui::RichText::new(s.field_title)
                .font(font_semibold(10.5))
                .color(TEXT_MUTED),
        );
        c.add_space(4.0);
        let (r1, _) = c.allocate_exact_size(Vec2::new(inner.width(), 34.0), Sense::hover());
        c.painter()
            .rect_filled(r1, CornerRadius::same(8), GLASS_PILL_IDLE);
        c.painter().rect_stroke(
            r1,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, RIM_SUBTLE),
            StrokeKind::Inside,
        );
        let mut i1 =
            c.new_child(egui::UiBuilder::new().max_rect(r1.shrink2(Vec2::new(10.0, 6.0))));
        i1.add(
            egui::TextEdit::singleline(&mut dlg.title)
                .frame(false)
                .desired_width(f32::INFINITY),
        );

        c.add_space(12.0);
        c.label(
            egui::RichText::new(s.field_artist)
                .font(font_semibold(10.5))
                .color(TEXT_MUTED),
        );
        c.add_space(4.0);
        let (r2, _) = c.allocate_exact_size(Vec2::new(inner.width(), 34.0), Sense::hover());
        c.painter()
            .rect_filled(r2, CornerRadius::same(8), GLASS_PILL_IDLE);
        c.painter().rect_stroke(
            r2,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, RIM_SUBTLE),
            StrokeKind::Inside,
        );
        let mut i2 =
            c.new_child(egui::UiBuilder::new().max_rect(r2.shrink2(Vec2::new(10.0, 6.0))));
        i2.add(
            egui::TextEdit::singleline(&mut dlg.artist)
                .frame(false)
                .desired_width(f32::INFINITY),
        );

        if let Some(ref err) = dlg.error {
            c.add_space(6.0);
            c.label(
                egui::RichText::new(err)
                    .font(font_regular(11.5))
                    .color(Color32::from_rgb(235, 120, 120)),
            );
        }

        let mut close_now = false;
        let mut save_req = None;
        let footer = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - 38.0), inner.max);
        let mut f_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(footer)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if frosted_cta_button(&mut f_ui, None, s.save, 36.0).clicked() {
            save_req = Some((dlg.path.clone(), dlg.title.clone(), dlg.artist.clone()));
        }
        f_ui.add_space(8.0);
        if glass_pill_button(&mut f_ui, None, s.cancel, 36.0, false).clicked() {
            close_now = true;
        }

        if let Some((path, title, artist)) = save_req {
            match rename_track(&path, &title, &artist) {
                Ok(new_path) => {
                    self.rename_dialog = None;
                    self.reload_library();
                    self.bpm.enqueue_if_needed(&new_path);
                    self.show_toast(s.toast_renamed);
                }
                Err(e) => {
                    if let Some(ref mut d) = self.rename_dialog {
                        d.error = Some(e);
                    }
                }
            }
        } else if close_now {
            self.rename_dialog = None;
        }
    }

    fn show_delete_dialog(&mut self, ui: &mut egui::Ui, screen: Rect) {
        let Some(ref dlg) = self.delete_dialog else {
            return;
        };
        let path = dlg.path.clone();
        let title = dlg.title.clone();
        let s = tr(self.config.language);

        ui.painter().rect_filled(
            screen,
            CornerRadius::ZERO,
            Color32::from_rgba_unmultiplied(0, 0, 0, 175),
        );
        let card = Rect::from_center_size(screen.center(), Vec2::new(400.0, 185.0));
        draw_liquid_glass_panel(ui.painter(), card, 18, GlassLevel::Modal);

        let inner = card.shrink2(Vec2::new(24.0, 20.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        c.label(
            egui::RichText::new(s.delete_confirm_title)
                .font(font_semibold(16.5))
                .color(TEXT_PRIMARY),
        );
        c.add_space(8.0);
        c.label(
            egui::RichText::new(format!("\"{title}\" — {}", s.delete_confirm_body))
                .font(font_regular(12.5))
                .color(TEXT_SECONDARY),
        );

        let footer = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - 38.0), inner.max);
        let mut f_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(footer)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let del_btn = f_ui
            .add(
                egui::Button::new(
                    egui::RichText::new(s.delete_btn)
                        .font(font_semibold(12.5))
                        .color(Color32::WHITE),
                )
                .fill(DANGER_GLASS)
                .corner_radius(CornerRadius::same(18))
                .min_size(Vec2::new(120.0, 36.0)),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);

        if del_btn.clicked() {
            if self.snap.current_path.as_ref() == Some(&path) {
                self.engine.send(AudioCommand::Stop);
            }
            if delete_track(&path).is_ok() {
                self.reload_library();
                self.show_toast(s.toast_deleted);
            }
            self.delete_dialog = None;
            return;
        }

        f_ui.add_space(8.0);
        if glass_pill_button(&mut f_ui, None, s.cancel, 36.0, false).clicked() {
            self.delete_dialog = None;
        }
    }

    fn draw_toast(&mut self, painter: &egui::Painter, screen: Rect) {
        let Some(ref toast) = self.toast else {
            return;
        };
        let elapsed = toast.created.elapsed().as_secs_f32();
        if elapsed > 2.8 {
            self.toast = None;
            return;
        }
        let alpha = if elapsed < 0.2 {
            elapsed / 0.2
        } else if elapsed > 2.3 {
            (2.8 - elapsed) / 0.5
        } else {
            1.0
        }
        .clamp(0.0, 1.0);

        let galley =
            painter.layout_no_wrap(toast.message.clone(), font_semibold(12.5), TEXT_PRIMARY);
        let size = galley.size() + Vec2::new(30.0, 16.0);
        let rect = Rect::from_center_size(
            Pos2::new(
                screen.center().x,
                screen.top() + 58.0 - (1.0 - alpha) * 8.0,
            ),
            size,
        );

        painter.rect_filled(
            rect,
            CornerRadius::same(16),
            Color32::from_rgba_unmultiplied(16, 18, 23, (alpha * 242.0) as u8),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(16),
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(255, 255, 255, (alpha * 44.0) as u8),
            ),
            StrokeKind::Inside,
        );
        painter.galley(
            Pos2::new(
                rect.center().x - galley.size().x * 0.5,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            TEXT_PRIMARY.gamma_multiply(alpha),
        );
    }

    fn show_queue_popover(&mut self, ui: &mut egui::Ui, player_rect: Rect) {
        if !self.queue_open {
            return;
        }
        let s = tr(self.config.language);
        let pop_w = 340.0;
        let pop_h = 370.0;
        let card = Rect::from_min_size(
            Pos2::new(
                (player_rect.right() - pop_w).max(player_rect.left() + 12.0),
                (player_rect.top() - pop_h - 10.0).max(46.0),
            ),
            Vec2::new(pop_w, pop_h),
        );

        let mut close_queue = false;
        let mut clear_uq = false;
        let mut remove_uq_idx: Option<usize> = None;
        let mut play_now_path: Option<PathBuf> = None;

        egui::Area::new(egui::Id::new("queue_popover_area"))
            .order(egui::Order::Foreground)
            .fixed_pos(card.min)
            .show(ui.ctx(), |area_ui| {
                let (rect, _) =
                    area_ui.allocate_exact_size(card.size(), Sense::click());
                draw_liquid_glass_panel(area_ui.painter(), rect, 14, GlassLevel::Modal);

                let inner = rect.shrink2(Vec2::new(16.0, 14.0));
                let mut c = area_ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(inner)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                );

                // Header
                let (hdr_r, _) =
                    c.allocate_exact_size(Vec2::new(inner.width(), 28.0), Sense::hover());
                c.painter().text(
                    Pos2::new(hdr_r.left(), hdr_r.center().y),
                    Align2::LEFT_CENTER,
                    s.queue_title,
                    font_semibold(15.0),
                    TEXT_PRIMARY,
                );
                let mut hdr_right = c.new_child(
                    egui::UiBuilder::new()
                        .max_rect(hdr_r)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                if glass_icon_button(&mut hdr_right, Icon::Close, 24.0, 10.0, false, false)
                    .clicked()
                {
                    close_queue = true;
                }
                if !self.user_queue.is_empty() {
                    hdr_right.add_space(6.0);
                    if glass_pill_button(&mut hdr_right, None, s.clear_queue, 24.0, false).clicked()
                    {
                        clear_uq = true;
                    }
                }

                c.add_space(8.0);

                egui::ScrollArea::vertical()
                    .id_salt("queue_pop_scroll")
                    .max_height(inner.height() - 38.0)
                    .auto_shrink([false, false])
                    .show(&mut c, |scroll| {
                        let row_w = scroll.available_width();
                        if !self.user_queue.is_empty() {
                            scroll.label(
                                egui::RichText::new(s.user_queue)
                                    .font(font_semibold(10.5))
                                    .color(TEXT_MUTED),
                            );
                            scroll.add_space(4.0);
                            let uq_clone: Vec<PathBuf> = self.user_queue.iter().cloned().collect();
                            for (i, p) in uq_clone.iter().enumerate() {
                                let (title, artist) = self
                                    .find_track(p)
                                    .map(|t| {
                                        (
                                            t.title.clone(),
                                            t.display_artist(s.unknown_artist).to_string(),
                                        )
                                    })
                                    .unwrap_or_else(|| ("Track".to_string(), String::new()));

                                let (r, resp) = scroll
                                    .allocate_exact_size(Vec2::new(row_w, 42.0), Sense::click());
                                if resp.hovered() {
                                    scroll.painter().rect_filled(
                                        r,
                                        CornerRadius::same(7),
                                        GLASS_PILL_IDLE,
                                    );
                                }
                                crate::ui::theme::draw_truncated_text(
                                    scroll.painter(),
                                    Rect::from_min_max(
                                        Pos2::new(r.left() + 8.0, r.top() + 4.0),
                                        Pos2::new(r.right() - 32.0, r.center().y + 1.0),
                                    ),
                                    &title,
                                    font_semibold(12.5),
                                    TEXT_PRIMARY,
                                    Align2::LEFT_CENTER,
                                );
                                crate::ui::theme::draw_truncated_text(
                                    scroll.painter(),
                                    Rect::from_min_max(
                                        Pos2::new(r.left() + 8.0, r.center().y + 1.0),
                                        Pos2::new(r.right() - 32.0, r.bottom() - 3.0),
                                    ),
                                    &artist,
                                    font_regular(11.0),
                                    TEXT_SECONDARY,
                                    Align2::LEFT_CENTER,
                                );

                                let rm_rect = Rect::from_center_size(
                                    Pos2::new(r.right() - 16.0, r.center().y),
                                    Vec2::splat(22.0),
                                );
                                let rm_resp = scroll.interact(
                                    rm_rect,
                                    scroll.id().with(("rm_uq", i)),
                                    Sense::click(),
                                );
                                crate::ui::icons::draw_icon(
                                    scroll.painter(),
                                    rm_rect.center(),
                                    9.5,
                                    Icon::Close,
                                    if rm_resp.hovered() {
                                        TEXT_PRIMARY
                                    } else {
                                        TEXT_MUTED
                                    },
                                );
                                if rm_resp.clicked() {
                                    remove_uq_idx = Some(i);
                                } else if resp.clicked() {
                                    remove_uq_idx = Some(i);
                                    play_now_path = Some(p.clone());
                                }
                            }
                            scroll.add_space(10.0);
                        }

                        scroll.label(
                            egui::RichText::new(s.up_next)
                                .font(font_semibold(10.5))
                                .color(TEXT_MUTED),
                        );
                        scroll.add_space(4.0);

                        let upcoming: Vec<PathBuf> = if !self.play_queue.is_empty()
                            && self.queue_pos + 1 < self.play_queue.len()
                        {
                            self.play_queue[(self.queue_pos + 1)..]
                                .iter()
                                .take(15)
                                .cloned()
                                .collect()
                        } else {
                            Vec::new()
                        };

                        if upcoming.is_empty() && self.user_queue.is_empty() {
                            scroll.add_space(16.0);
                            scroll.label(
                                egui::RichText::new(s.queue_empty)
                                    .font(font_regular(12.0))
                                    .color(TEXT_MUTED),
                            );
                        } else {
                            for p in upcoming {
                                let (title, artist) = self
                                    .find_track(&p)
                                    .map(|t| {
                                        (
                                            t.title.clone(),
                                            t.display_artist(s.unknown_artist).to_string(),
                                        )
                                    })
                                    .unwrap_or_else(|| ("Track".to_string(), String::new()));

                                let (r, resp) = scroll
                                    .allocate_exact_size(Vec2::new(row_w, 40.0), Sense::click());
                                if resp.hovered() {
                                    scroll.painter().rect_filled(
                                        r,
                                        CornerRadius::same(7),
                                        GLASS_PILL_IDLE,
                                    );
                                }
                                crate::ui::theme::draw_truncated_text(
                                    scroll.painter(),
                                    Rect::from_min_max(
                                        Pos2::new(r.left() + 8.0, r.top() + 4.0),
                                        Pos2::new(r.right() - 8.0, r.center().y + 1.0),
                                    ),
                                    &title,
                                    font_semibold(12.5),
                                    TEXT_PRIMARY,
                                    Align2::LEFT_CENTER,
                                );
                                crate::ui::theme::draw_truncated_text(
                                    scroll.painter(),
                                    Rect::from_min_max(
                                        Pos2::new(r.left() + 8.0, r.center().y + 1.0),
                                        Pos2::new(r.right() - 8.0, r.bottom() - 3.0),
                                    ),
                                    &artist,
                                    font_regular(11.0),
                                    TEXT_SECONDARY,
                                    Align2::LEFT_CENTER,
                                );
                                if resp.clicked() {
                                    play_now_path = Some(p);
                                }
                            }
                        }
                    });
            });

        if close_queue {
            self.queue_open = false;
        }
        if clear_uq {
            self.user_queue.clear();
            self.sync_next_preload();
        }
        if let Some(idx) = remove_uq_idx {
            self.user_queue.remove(idx);
            self.sync_next_preload();
        }
        if let Some(path) = play_now_path {
            if let Some(pos) = self.play_queue.iter().position(|p| p == &path) {
                self.queue_pos = pos;
            }
            if let Some(spec) = self.make_track_spec(&path) {
                let (next_path, from_uq) = self.peek_next_path();
                self.next_came_from_user_queue = from_uq;
                let next_spec = next_path.as_deref().and_then(|p| self.make_track_spec(p));
                self.engine.send(AudioCommand::PlayNow {
                    track: spec,
                    next: next_spec,
                    mode: self.config.playback_mode,
                });
            }
        }
    }
}

impl eframe::App for RustifyApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if let Some(ref out_path) = self.capture_path.clone() {
            let mut saved = false;
            ctx.input(|i| {
                for ev in &i.events {
                    if let egui::Event::Screenshot { image, .. } = ev {
                        let w = image.width() as u32;
                        let h = image.height() as u32;
                        let rgba: Vec<u8> = image
                            .pixels
                            .iter()
                            .flat_map(|c| c.to_srgba_unmultiplied())
                            .collect();
                        if let Some(buf) = image::RgbaImage::from_raw(w, h, rgba) {
                            let _ = buf.save(out_path);
                        }
                        saved = true;
                    }
                }
            });
            if saved {
                ctx.send_viewport_cmd(ViewportCommand::Close);
                return;
            }
            self.capture_frames += 1;
            ctx.request_repaint();
            let wait_frames = if self.config.setup_completed {
                90
            } else {
                20
            };
            if self.capture_frames == wait_frames {
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));
            }
        }

        self.poll_background_events();
        self.handle_keyboard_shortcuts(ctx);
        let dragging_files = self.handle_file_drops(ctx);

        let time = self.start_instant.elapsed().as_secs_f32();
        let shader_time = if self.config.animated_glass_backdrop {
            time
        } else {
            12.0
        };
        let mix_intensity = if self.snap.mix.active {
            (self.snap.mix.progress * std::f32::consts::PI).sin()
        } else {
            0.0
        };

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::TRANSPARENT))
            .show(ctx, |ui| {
                let screen = ui.max_rect();

                // 1. Pitch-black obsidian Liquid Glass backdrop with architectural technical grid
                if let Some(ref bg) = self.backdrop {
                    bg.paint(
                        ui.painter(),
                        screen,
                        shader_time,
                        self.snap.bass_energy,
                        mix_intensity,
                    );
                } else {
                    ui.painter()
                        .rect_filled(screen, CornerRadius::ZERO, Color32::from_rgb(5, 5, 7));
                }

                ui.painter().rect_stroke(
                    screen,
                    CornerRadius::same(10),
                    Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 18),
                    ),
                    StrokeKind::Inside,
                );

                self.handle_window_resize_edges(ctx, screen);

                if self.mini_mode {
                    self.draw_mini_widget(ui, screen);
                    return;
                }

                // 2. Frameless Custom Titlebar
                let titlebar_h = 36.0;
                let titlebar_rect = Rect::from_min_max(
                    screen.min,
                    Pos2::new(screen.right(), screen.top() + titlebar_h),
                );
                self.draw_titlebar(ui, titlebar_rect);

                let content_rect = Rect::from_min_max(
                    Pos2::new(screen.left(), screen.top() + titlebar_h),
                    screen.max,
                );

                // 3. First-Run Setup Wizard OR Main Workspace
                if !self.config.setup_completed {
                    let mut setup_ui =
                        ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
                    if show_setup_wizard(
                        &mut setup_ui,
                        &mut self.config,
                        &mut self.setup_state,
                        self.logo_tex.as_ref(),
                    ) {
                        self.reload_library();
                    }
                    return;
                }

                let pad = 12.0;
                let player_h = 74.0;

                let player_rect = Rect::from_min_max(
                    Pos2::new(
                        content_rect.left() + pad,
                        content_rect.bottom() - player_h - pad,
                    ),
                    Pos2::new(content_rect.right() - pad, content_rect.bottom() - pad),
                );

                let main_top = content_rect.top() + 4.0;
                let main_bottom = player_rect.top() - pad;
                let sidebar_w = (content_rect.width() * 0.19).clamp(195.0, 235.0);

                let sidebar_rect = Rect::from_min_max(
                    Pos2::new(content_rect.left() + pad, main_top),
                    Pos2::new(content_rect.left() + pad + sidebar_w, main_bottom),
                );

                let playlist_left = sidebar_rect.right() + pad;
                let playlist_rect = Rect::from_min_max(
                    Pos2::new(playlist_left, main_top),
                    Pos2::new(content_rect.right() - pad, main_bottom),
                );

                // --- SIDEBAR ---
                let bpm_prog = self.bpm.progress();
                if let Some(sb_act) = show_sidebar(
                    ui,
                    sidebar_rect,
                    self.config.language,
                    &self.playlists,
                    &self.selection,
                    bpm_prog,
                    &mut self.creating_playlist,
                    &mut self.new_playlist_name,
                    &mut self.visualizer,
                ) {
                    let s = tr(self.config.language);
                    match sb_act {
                        SidebarAction::Select(sel) => {
                            let ViewSelection::Playlist(idx) = sel;
                            if let Some(pl) = self.playlists.get(idx) {
                                self.config.last_playlist = Some(pl.name.clone());
                                self.config.save();
                            }
                            self.selection = sel;
                            self.search_query.clear();
                        }
                        SidebarAction::CreatePlaylist(name) => {
                            if let Some(ref dir) = self.config.music_dir {
                                if create_playlist(dir, &name).is_ok() {
                                    self.reload_library();
                                    if let Some(idx) = self
                                        .playlists
                                        .iter()
                                        .position(|p| p.name.eq_ignore_ascii_case(&name))
                                    {
                                        self.selection = ViewSelection::Playlist(idx);
                                    }
                                    self.show_toast(format!(
                                        "{} — {name}",
                                        s.toast_playlist_created
                                    ));
                                }
                            }
                        }
                        SidebarAction::OpenImportModal => {
                            self.import_modal.open_from_sidebar();
                        }
                        SidebarAction::ToggleVisualizer => {
                            self.visualizer.open = !self.visualizer.open;
                        }
                        SidebarAction::VisualizerConfigChanged => {}
                        SidebarAction::OpenSettings => {
                            self.settings_open = true;
                        }
                    }
                }

                // --- MAIN PLAYLIST VIEW OR SYNCHRONIZED LYRICS VIEW ---
                let (view_title, is_real_pl, _pl_dir, tracks) = self.collect_view_tracks();
                let display_indices = self.build_display_indices(&tracks);
                let total_duration_sec: f32 =
                    tracks.iter().map(|t| t.effective_duration()).sum();

                let cur_track = self
                    .snap
                    .current_path
                    .as_deref()
                    .and_then(|p| self.find_track(p))
                    .cloned();

                if self.lyrics_open {
                    if let Some(ref ct) = cur_track {
                        self.lyrics_mgr.ensure_fetched(ct);
                    }
                    if let Some(l_act) = show_lyrics_panel(
                        ui,
                        playlist_rect,
                        self.config.language,
                        cur_track.as_ref(),
                        self.snap.position_sec,
                        &mut self.lyrics_mgr,
                    ) {
                        match l_act {
                            LyricsAction::SeekTo(sec) => {
                                self.engine.send(AudioCommand::Seek(sec));
                            }
                            LyricsAction::Close => {
                                self.lyrics_open = false;
                            }
                            LyricsAction::RetryFetch => {
                                if let Some(ref ct) = cur_track {
                                    self.lyrics_mgr.force_refetch(ct);
                                }
                            }
                        }
                    }
                } else {
                    let pl_info = PlaylistViewInfo {
                        title: &view_title,
                        is_real_playlist: is_real_pl,
                        tracks: &tracks,
                        display_indices: &display_indices,
                        total_duration_sec,
                    };

                    if let Some(pl_act) = show_playlist_view(
                        ui,
                        playlist_rect,
                        &self.config,
                        &pl_info,
                        &mut self.covers,
                        &mut self.search_query,
                        &mut self.focus_search,
                        self.snap.current_path.as_ref(),
                        self.snap.is_playing,
                        time,
                        self.snap.bass_energy,
                        dragging_files,
                    ) {
                        let s = tr(self.config.language);
                        match pl_act {
                            PlaylistAction::PlayTrackAt(idx) => {
                                self.play_track_from_view(&tracks, idx);
                            }
                            PlaylistAction::TogglePlayPlaylist => {
                                let pl_active = self
                                    .snap
                                    .current_path
                                    .as_ref()
                                    .map(|cp| tracks.iter().any(|t| &t.path == cp))
                                    .unwrap_or(false);
                                if pl_active {
                                    self.engine.send(AudioCommand::TogglePlay);
                                } else if let Some(&first_idx) = display_indices.first() {
                                    self.play_track_from_view(&tracks, first_idx);
                                }
                            }
                            PlaylistAction::SetMode(mode) => {
                                self.config.playback_mode = mode;
                                self.config.save();
                                self.rebuild_active_queue_for_mode();
                            }
                            PlaylistAction::FetchMissingCovers => {
                                let count = self.covers.fetch_missing_for_tracks(&tracks);
                                if count > 0 {
                                    self.show_toast(s.toast_covers_fetching);
                                } else {
                                    self.show_toast(s.toast_covers_all_present);
                                }
                            }
                            PlaylistAction::PickCustomCover(path) => {
                                if let Some(picked) = rfd::FileDialog::new()
                                    .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
                                    .pick_file()
                                {
                                    if set_custom_cover(&path, &picked).is_ok() {
                                        self.covers.invalidate(&path);
                                        self.show_toast(s.toast_cover_updated);
                                    }
                                }
                            }
                            PlaylistAction::RefreshCoverOnline(path) => {
                                if let Some(t) = self.find_track(&path).cloned() {
                                    self.covers.force_fetch_one(&t);
                                    self.show_toast(s.toast_covers_fetching);
                                }
                            }
                            PlaylistAction::PlayNext(path) => {
                                self.user_queue.push_front(path);
                                self.sync_next_preload();
                                self.show_toast(s.toast_play_next);
                            }
                            PlaylistAction::AddToQueue(path) => {
                                self.user_queue.push_back(path);
                                self.sync_next_preload();
                                self.show_toast(s.toast_added_queue);
                            }
                            PlaylistAction::OpenRename(path, title, artist) => {
                                self.rename_dialog = Some(RenameDialog {
                                    path,
                                    title,
                                    artist,
                                    error: None,
                                });
                            }
                            PlaylistAction::ReanalyzeBpm(path) => {
                                self.bpm.force_enqueue(&path);
                            }
                            PlaylistAction::RevealInExplorer(path) => {
                                reveal_in_explorer(&path);
                            }
                            PlaylistAction::RequestDelete(path, title) => {
                                self.delete_dialog = Some(DeleteDialog { path, title });
                            }
                        }
                    }
                }

                // --- FLOATING LIQUID GLASS PLAYER BAR ---
                let (next_path, _) = self.peek_next_path();
                let next_track = next_path
                    .as_deref()
                    .and_then(|p| self.find_track(p))
                    .cloned();
                let current_cover = cur_track
                    .as_ref()
                    .and_then(|t| self.covers.get_texture(ui.ctx(), &t.path));

                if let Some(p_act) = show_player_bar(
                    ui,
                    player_rect,
                    &self.config,
                    cur_track.as_ref(),
                    next_track.as_ref(),
                    &self.snap,
                    current_cover.as_ref(),
                    self.lyrics_open,
                    self.queue_open,
                    self.user_queue.len(),
                ) {
                    match p_act {
                        PlayerAction::TogglePlay => {
                            if self.snap.current_path.is_some() {
                                self.engine.send(AudioCommand::TogglePlay);
                            } else if let Some(&first_idx) = display_indices.first() {
                                self.play_track_from_view(&tracks, first_idx);
                            }
                        }
                        PlayerAction::Prev => self.play_prev(),
                        PlayerAction::Next => self.play_next(true),
                        PlayerAction::Seek(sec) => self.engine.send(AudioCommand::Seek(sec)),
                        PlayerAction::CycleMode => {
                            self.config.playback_mode = match self.config.playback_mode {
                                PlaybackMode::Normal => PlaybackMode::Shuffle,
                                PlaybackMode::Shuffle => PlaybackMode::DjMix,
                                PlaybackMode::DjMix => PlaybackMode::Normal,
                            };
                            self.config.save();
                            self.rebuild_active_queue_for_mode();
                        }
                        PlayerAction::CycleRepeat => {
                            self.config.repeat_mode = self.config.repeat_mode.next();
                            self.config.save();
                            self.sync_next_preload();
                        }
                        PlayerAction::ToggleLyrics => {
                            self.lyrics_open = !self.lyrics_open;
                            if self.lyrics_open {
                                if let Some(ref ct) = cur_track {
                                    self.lyrics_mgr.ensure_fetched(ct);
                                }
                            }
                        }
                        PlayerAction::ToggleQueuePopup => {
                            self.queue_open = !self.queue_open;
                        }
                        PlayerAction::EnterMiniMode => {
                            let ctx_c = ui.ctx().clone();
                            self.set_mini_mode(&ctx_c, true);
                        }
                        PlayerAction::SetVolume(v) => {
                            self.config.volume = v.clamp(0.0, 1.0);
                            self.config.muted = v <= 0.001;
                            self.engine.send(AudioCommand::SetVolume(self.config.volume));
                            self.config.save();
                        }
                        PlayerAction::ToggleMute => {
                            self.config.muted = !self.config.muted;
                            let v = if self.config.muted {
                                0.0
                            } else {
                                self.config.volume
                            };
                            self.engine.send(AudioCommand::SetVolume(v));
                            self.config.save();
                        }
                    }
                }

                if self.queue_open {
                    self.show_queue_popover(ui, player_rect);
                }

                // --- OVERLAY MODALS ---
                if let Some(imp_act) = show_playlist_import_modal(
                    ui,
                    screen,
                    &self.config,
                    &mut self.import_modal,
                ) {
                    let s = tr(self.config.language);
                    match imp_act {
                        PlaylistImportAction::ImportPcFolder(folder) => {
                            if let Some(ref music_dir) = self.config.music_dir.clone() {
                                match import_pc_folder_as_playlist(music_dir, &folder) {
                                    Ok((pl_name, count)) => {
                                        self.reload_library();
                                        if let Some(idx) = self
                                            .playlists
                                            .iter()
                                            .position(|p| p.name.eq_ignore_ascii_case(&pl_name))
                                        {
                                            self.selection = ViewSelection::Playlist(idx);
                                        }
                                        self.show_toast(format!(
                                            "\"{pl_name}\" ({count} {})",
                                            s.toast_imported_files
                                        ));
                                    }
                                    Err(err) => {
                                        self.show_toast(err);
                                    }
                                }
                            }
                        }
                        PlaylistImportAction::CreateEmptyPlaylist(name) => {
                            if let Some(ref dir) = self.config.music_dir {
                                if create_playlist(dir, &name).is_ok() {
                                    self.reload_library();
                                    if let Some(idx) = self
                                        .playlists
                                        .iter()
                                        .position(|p| p.name.eq_ignore_ascii_case(&name))
                                    {
                                        self.selection = ViewSelection::Playlist(idx);
                                    }
                                    self.show_toast(format!(
                                        "{} — {name}",
                                        s.toast_playlist_created
                                    ));
                                }
                            }
                        }
                    }
                }

                if let Some(set_act) = show_settings_modal(
                    ui,
                    screen,
                    &mut self.settings_open,
                    &mut self.config,
                    self.discord_rpc.status(),
                ) {
                    let s = tr(self.config.language);
                    match set_act {
                        SettingsAction::MusicFolderChanged => {
                            self.reload_library();
                            self.show_toast(s.toast_library_refreshed);
                        }
                        SettingsAction::DjSettingsChanged => {
                            self.engine.send(AudioCommand::SetDjSettings(DjSettings {
                                crossfade_secs: self.config.dj_crossfade_secs,
                                tempo_sync: self.config.dj_tempo_sync,
                                bass_swap: self.config.dj_bass_swap,
                                trim_silence: self.config.dj_trim_silence,
                                normal_short_fade: self.config.normal_short_fade,
                            }));
                        }
                        SettingsAction::EqChanged => {
                            self.engine.send(AudioCommand::SetEq(self.config.eq_bands_db));
                        }
                        SettingsAction::LoudnessNormChanged => {
                            self.engine.send(AudioCommand::SetLoudnessNorm(
                                self.config.loudness_normalization,
                            ));
                        }
                        SettingsAction::DiscordRpcChanged => {
                            self.discord_rpc.configure(
                                self.config.discord_rpc_enabled,
                                self.config.discord_client_id.clone(),
                            );
                        }
                        SettingsAction::ReanalyzeAllBpm => {
                            let all_paths: Vec<PathBuf> = self
                                .playlists
                                .iter()
                                .flat_map(|p| p.tracks.iter().map(|t| t.path.clone()))
                                .collect();
                            for pl in &mut self.playlists {
                                for t in &mut pl.tracks {
                                    t.analysis = None;
                                }
                            }
                            self.bpm.clear_cache_and_reanalyze(&all_paths);
                            self.show_toast(s.toast_bpm_restarted);
                        }
                    }
                }

                self.show_rename_dialog(ui, screen);
                self.show_delete_dialog(ui, screen);
                self.draw_toast(ui.painter(), screen);
            });

        self.visualizer.show_window(ctx, &self.engine);

        let need_fast = self.snap.is_playing
            || self.visualizer.open
            || self.config.animated_glass_backdrop
            || self.bpm.is_analyzing()
            || self.covers.progress().is_some()
            || self.lyrics_mgr.is_loading()
            || self.toast.is_some();

        if need_fast {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn on_exit(&mut self, gl: Option<&egui_glow::glow::Context>) {
        self.discord_rpc.shutdown();
        self.bpm.save_cache();
        if self.capture_path.is_none() {
            self.config.save();
        }
        if let (Some(bg), Some(gl)) = (&self.backdrop, gl) {
            bg.destroy(gl);
        }
    }
}

fn load_logo_texture(ctx: &Context) -> Option<TextureHandle> {
    let img = image::load_from_memory(LOGO_PNG).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let color_image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
    Some(ctx.load_texture("rustify_logo", color_image, TextureOptions::LINEAR))
}
