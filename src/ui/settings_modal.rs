use crate::config::{AppConfig, EqPreset};
use crate::discord_rpc::RpcStatus;
use crate::i18n::{tr, Language};
use crate::ui::icons::Icon;
use crate::ui::theme::{
    draw_liquid_glass_panel, draw_truncated_text, font_regular, font_semibold, glass_icon_button,
    glass_pill_button, glass_slider, glass_toggle, rgba, GlassLevel, FROST_CTA_IDLE,
    FROST_CTA_TEXT, GLASS_PILL_IDLE, RIM_BRIGHT, RIM_SUBTLE, TEXT_MUTED, TEXT_PRIMARY,
    TEXT_SECONDARY,
};
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

pub enum SettingsAction {
    MusicFolderChanged,
    DjSettingsChanged,
    EqChanged,
    LoudnessNormChanged,
    DiscordRpcChanged,
    ReanalyzeAllBpm,
}

pub fn show_settings_modal(
    ui: &mut Ui,
    screen: Rect,
    open: &mut bool,
    config: &mut AppConfig,
    rpc_status: RpcStatus,
) -> Option<SettingsAction> {
    if !*open {
        return None;
    }
    let s = tr(config.language);
    let mut action = None;

    let scrim_resp = ui.interact(screen, ui.id().with("settings_scrim"), Sense::click());
    ui.painter().rect_filled(
        screen,
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(0, 0, 0, 180),
    );

    let card_w = 516.0f32.min(screen.width() - 40.0);
    let card_h = 600.0f32.min(screen.height() - 36.0);
    let card_rect = Rect::from_center_size(screen.center(), Vec2::new(card_w, card_h));

    if scrim_resp.clicked()
        && !card_rect.contains(scrim_resp.interact_pointer_pos().unwrap_or(Pos2::ZERO))
    {
        *open = false;
        return None;
    }

    draw_liquid_glass_panel(ui.painter(), card_rect, 14, GlassLevel::Modal);

    let inner = card_rect.shrink2(Vec2::new(22.0, 20.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );

    // Header
    child.horizontal(|ui| {
        ui.label(
            egui::RichText::new(s.settings_title)
                .font(font_semibold(17.0))
                .color(TEXT_PRIMARY),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if glass_icon_button(ui, Icon::Close, 26.0, 11.0, false, false).clicked() {
                *open = false;
            }
        });
    });

    child.add_space(12.0);

    egui::ScrollArea::vertical()
        .id_salt("settings_modal_scroll")
        .auto_shrink([false, false])
        .show(&mut child, |scroll| {
            let w = scroll.available_width();

    // Row 1: Language [ Deutsch | English ]
    draw_setting_card(scroll, w, 48.0, |ui, rect| {
        ui.painter().text(
            Pos2::new(rect.left() + 14.0, rect.center().y),
            Align2::LEFT_CENTER,
            s.settings_language,
            font_semibold(13.0),
            TEXT_PRIMARY,
        );

        let seg_w = 72.0;
        let group_rect = Rect::from_min_size(
            Pos2::new(rect.right() - seg_w * 2.0 - 16.0, rect.center().y - 15.0),
            Vec2::new(seg_w * 2.0 + 4.0, 30.0),
        );
        ui.painter()
            .rect_filled(group_rect, CornerRadius::same(8), Color32::from_rgba_unmultiplied(0, 0, 0, 90));
        ui.painter().rect_stroke(
            group_rect,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, RIM_SUBTLE),
            StrokeKind::Inside,
        );

        for (i, lang) in [Language::De, Language::En].iter().enumerate() {
            let sel = config.language == *lang;
            let sr = Rect::from_min_size(
                Pos2::new(group_rect.left() + 2.0 + i as f32 * seg_w, group_rect.top() + 2.0),
                Vec2::new(seg_w, 26.0),
            );
            let resp = ui
                .interact(sr, ui.id().with(("set_lang", i)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if sel {
                ui.painter().rect_filled(sr, CornerRadius::same(6), FROST_CTA_IDLE);
            }
            ui.painter().text(
                sr.center(),
                Align2::CENTER_CENTER,
                lang.label(),
                font_semibold(11.5),
                if sel { FROST_CTA_TEXT } else { TEXT_SECONDARY },
            );
            if resp.clicked() && !sel {
                config.language = *lang;
                config.save();
            }
        }
    });

    scroll.add_space(8.0);

    // Row 2: Music Folder
    draw_setting_card(scroll, w, 54.0, |ui, rect| {
        let p = ui.painter();
        p.text(
            Pos2::new(rect.left() + 14.0, rect.top() + 16.0),
            Align2::LEFT_CENTER,
            s.settings_music_folder,
            font_semibold(13.0),
            TEXT_PRIMARY,
        );
        let dir_str = config
            .music_dir
            .as_ref()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| s.setup_no_folder.to_string());
        draw_truncated_text(
            p,
            Rect::from_min_max(
                Pos2::new(rect.left() + 14.0, rect.top() + 28.0),
                Pos2::new(rect.right() - 138.0, rect.bottom() - 6.0),
            ),
            &dir_str,
            font_regular(11.5),
            TEXT_SECONDARY,
            Align2::LEFT_CENTER,
        );

        let btn_rect = Rect::from_min_size(
            Pos2::new(rect.right() - 126.0, rect.center().y - 15.0),
            Vec2::new(114.0, 30.0),
        );
        let mut b_ui = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect));
        if glass_pill_button(&mut b_ui, None, s.settings_change_folder, 30.0, false).clicked() {
            let mut dialog = rfd::FileDialog::new();
            if let Some(ref cur) = config.music_dir {
                dialog = dialog.set_directory(cur);
            }
            if let Some(picked) = dialog.pick_folder() {
                config.music_dir = Some(picked);
                config.save();
                action = Some(SettingsAction::MusicFolderChanged);
            }
        }
    });

    scroll.add_space(8.0);

    // Row 3: DJ Crossfade Duration [ 4s | 6s | 8s | 10s | 12s ]
    draw_setting_card(scroll, w, 48.0, |ui, rect| {
        ui.painter().text(
            Pos2::new(rect.left() + 14.0, rect.center().y),
            Align2::LEFT_CENTER,
            s.settings_crossfade_len,
            font_semibold(13.0),
            TEXT_PRIMARY,
        );

        let steps = [4.0f32, 6.0, 8.0, 10.0, 12.0];
        let seg_w = 36.0;
        let total_w = seg_w * steps.len() as f32 + 4.0;
        let group_rect = Rect::from_min_size(
            Pos2::new(rect.right() - total_w - 12.0, rect.center().y - 15.0),
            Vec2::new(total_w, 30.0),
        );
        ui.painter()
            .rect_filled(group_rect, CornerRadius::same(8), Color32::from_rgba_unmultiplied(0, 0, 0, 90));
        ui.painter().rect_stroke(
            group_rect,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, RIM_SUBTLE),
            StrokeKind::Inside,
        );

        let current_rounded = config.dj_crossfade_secs.round();
        for (i, &sec) in steps.iter().enumerate() {
            let sel = (current_rounded - sec).abs() < 1.1;
            let sr = Rect::from_min_size(
                Pos2::new(group_rect.left() + 2.0 + i as f32 * seg_w, group_rect.top() + 2.0),
                Vec2::new(seg_w, 26.0),
            );
            let resp = ui
                .interact(sr, ui.id().with(("dj_sec", i)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if sel {
                ui.painter().rect_filled(sr, CornerRadius::same(6), FROST_CTA_IDLE);
            }
            ui.painter().text(
                sr.center(),
                Align2::CENTER_CENTER,
                format!("{sec:.0}s"),
                font_semibold(11.5),
                if sel { FROST_CTA_TEXT } else { TEXT_SECONDARY },
            );
            if resp.clicked() && !sel {
                config.dj_crossfade_secs = sec;
                config.dj_tempo_sync = true;
                config.dj_bass_swap = true;
                config.dj_trim_silence = true;
                config.save();
                action = Some(SettingsAction::DjSettingsChanged);
            }
        }
    });

    scroll.add_space(8.0);

    // Row 4: BPM & Harmonic Key Re-Analyze
    draw_setting_card(scroll, w, 46.0, |ui, rect| {
        ui.painter().text(
            Pos2::new(rect.left() + 14.0, rect.center().y),
            Align2::LEFT_CENTER,
            "BPM, Beat-Grid & Tonart",
            font_semibold(13.0),
            TEXT_PRIMARY,
        );

        let btn_rect = Rect::from_min_size(
            Pos2::new(rect.right() - 188.0, rect.center().y - 14.0),
            Vec2::new(176.0, 28.0),
        );
        let mut b_ui = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect));
        if glass_pill_button(&mut b_ui, None, s.settings_reanalyze_all, 28.0, false).clicked() {
            action = Some(SettingsAction::ReanalyzeAllBpm);
        }
    });

    scroll.add_space(10.0);

    // Row 5: Real-Time 5-Band Equalizer & Bass Control
    let is_de = config.language == Language::De;
    draw_setting_card(scroll, w, 214.0, |ui, rect| {
        let eq_inner = rect.shrink2(Vec2::new(14.0, 12.0));
        let mut eq_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(eq_inner)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );

        // Header + Reset button
        let (hdr_r, _) =
            eq_ui.allocate_exact_size(Vec2::new(eq_inner.width(), 24.0), Sense::hover());
        eq_ui.painter().text(
            Pos2::new(hdr_r.left(), hdr_r.center().y),
            Align2::LEFT_CENTER,
            if is_de {
                "Equalizer & Bass-Steuerung"
            } else {
                "Equalizer & Bass Control"
            },
            font_semibold(13.0),
            TEXT_PRIMARY,
        );
        let reset_r = Rect::from_min_size(
            Pos2::new(hdr_r.right() - 78.0, hdr_r.top()),
            Vec2::new(78.0, 24.0),
        );
        let mut r_ui = eq_ui.new_child(egui::UiBuilder::new().max_rect(reset_r));
        if glass_pill_button(
            &mut r_ui,
            None,
            if is_de { "Reset" } else { "Reset" },
            24.0,
            false,
        )
        .clicked()
        {
            config.eq_preset = EqPreset::Flat;
            config.eq_bands_db = EqPreset::Flat.bands_db();
            config.save();
            action = Some(SettingsAction::EqChanged);
        }

        eq_ui.add_space(6.0);

        // EQ Preset Pills
        let presets = [
            EqPreset::Flat,
            EqPreset::BassBoost,
            EqPreset::HardstyleKick,
            EqPreset::VocalClarity,
            EqPreset::RockPop,
        ];
        eq_ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            for preset in presets {
                let active = config.eq_preset == preset;
                let lbl = match preset {
                    EqPreset::Flat => "Flat",
                    EqPreset::BassBoost => "Bass Boost",
                    EqPreset::HardstyleKick => "Hardstyle",
                    EqPreset::VocalClarity => {
                        if is_de {
                            "Stimmen"
                        } else {
                            "Vocals"
                        }
                    }
                    EqPreset::RockPop => "Pop / Rock",
                    EqPreset::Custom => "Custom",
                };
                let (pr, presp) = ui.allocate_exact_size(Vec2::new(82.0, 24.0), Sense::click());
                if active {
                    ui.painter()
                        .rect_filled(pr, CornerRadius::same(6), FROST_CTA_IDLE);
                } else {
                    ui.painter().rect_filled(
                        pr,
                        CornerRadius::same(6),
                        if presp.hovered() {
                            rgba(255, 255, 255, 18)
                        } else {
                            rgba(255, 255, 255, 8)
                        },
                    );
                    ui.painter().rect_stroke(
                        pr,
                        CornerRadius::same(6),
                        Stroke::new(1.0_f32, if presp.hovered() { RIM_BRIGHT } else { RIM_SUBTLE }),
                        StrokeKind::Inside,
                    );
                }
                ui.painter().text(
                    pr.center(),
                    Align2::CENTER_CENTER,
                    lbl,
                    font_semibold(10.5),
                    if active {
                        FROST_CTA_TEXT
                    } else {
                        TEXT_SECONDARY
                    },
                );
                if presp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    config.eq_preset = preset;
                    config.eq_bands_db = preset.bands_db();
                    config.save();
                    action = Some(SettingsAction::EqChanged);
                }
            }
        });

        eq_ui.add_space(10.0);

        // 5 Frequency Band Sliders (-12 dB .. +12 dB)
        let bands: [(&str, &str); 5] = if is_de {
            [
                ("60 Hz", "Sub-Bass"),
                ("230 Hz", "Bass"),
                ("910 Hz", "Mitten"),
                ("3.6 kHz", "Präsenz"),
                ("14 kHz", "Höhen"),
            ]
        } else {
            [
                ("60 Hz", "Sub-Bass"),
                ("230 Hz", "Bass"),
                ("910 Hz", "Mids"),
                ("3.6 kHz", "Presence"),
                ("14 kHz", "Treble"),
            ]
        };

        for (idx, (freq, name)) in bands.iter().enumerate() {
            let (row_r, _) =
                eq_ui.allocate_exact_size(Vec2::new(eq_inner.width(), 24.0), Sense::hover());
            let p = eq_ui.painter();
            p.text(
                Pos2::new(row_r.left(), row_r.center().y),
                Align2::LEFT_CENTER,
                format!("{freq} · {name}"),
                font_regular(11.5),
                TEXT_SECONDARY,
            );

            let db = config.eq_bands_db[idx].clamp(-12.0, 12.0);
            let norm = (db + 12.0) / 24.0;
            let slider_r = Rect::from_min_max(
                Pos2::new(row_r.left() + 124.0, row_r.top() + 3.0),
                Pos2::new(row_r.right() - 56.0, row_r.bottom() - 3.0),
            );
            // Subtle 0 dB center reference tick
            p.line_segment(
                [
                    Pos2::new(slider_r.center().x, slider_r.top() + 3.0),
                    Pos2::new(slider_r.center().x, slider_r.bottom() - 3.0),
                ],
                Stroke::new(1.0_f32, rgba(255, 255, 255, 36)),
            );

            let mut s_ui = eq_ui.new_child(egui::UiBuilder::new().max_rect(slider_r));
            let (new_norm, _) = glass_slider(
                &mut s_ui,
                match idx {
                    0 => "eq_band_0",
                    1 => "eq_band_1",
                    2 => "eq_band_2",
                    3 => "eq_band_3",
                    _ => "eq_band_4",
                },
                slider_r.width(),
                18.0,
                norm,
                None,
                None,
            );
            if let Some(f) = new_norm {
                let raw_db = (f * 24.0 - 12.0).clamp(-12.0, 12.0);
                // Snap cleanly to 0.5 dB steps (and 0.0 when close to center)
                let snapped = if raw_db.abs() < 0.4 {
                    0.0
                } else {
                    (raw_db * 2.0).round() * 0.5
                };
                if (snapped - config.eq_bands_db[idx]).abs() > 0.01 {
                    config.eq_bands_db[idx] = snapped;
                    config.eq_preset = EqPreset::Custom;
                    config.save();
                    action = Some(SettingsAction::EqChanged);
                }
            }

            let val_str = if config.eq_bands_db[idx] > 0.05 {
                format!("+{:.1} dB", config.eq_bands_db[idx])
            } else {
                format!("{:.1} dB", config.eq_bands_db[idx])
            };
            eq_ui.painter().text(
                Pos2::new(row_r.right(), row_r.center().y),
                Align2::RIGHT_CENTER,
                val_str,
                font_semibold(11.0),
                if config.eq_bands_db[idx].abs() > 0.05 {
                    TEXT_PRIMARY
                } else {
                    TEXT_MUTED
                },
            );
        }
    });

    scroll.add_space(10.0);

    // Row 6: Loudness Normalization (-14 LUFS / ITU-R BS.1770-4)
    draw_setting_card(scroll, w, 58.0, |ui, rect| {
        let p = ui.painter();
        p.text(
            Pos2::new(rect.left() + 14.0, rect.top() + 19.0),
            Align2::LEFT_CENTER,
            if is_de {
                "Lautstärke-Normalisierung (-14 LUFS)"
            } else {
                "Loudness Normalization (-14 LUFS)"
            },
            font_semibold(13.0),
            TEXT_PRIMARY,
        );
        draw_truncated_text(
            p,
            Rect::from_min_max(
                Pos2::new(rect.left() + 14.0, rect.top() + 31.0),
                Pos2::new(rect.right() - 68.0, rect.bottom() - 6.0),
            ),
            if is_de {
                "ITU-R BS.1770-4 · Einheitliche Lautstärke ohne Clipping"
            } else {
                "ITU-R BS.1770-4 · Consistent loudness without clipping"
            },
            font_regular(11.0),
            TEXT_SECONDARY,
            Align2::LEFT_CENTER,
        );

        let toggle_rect = Rect::from_min_size(
            Pos2::new(rect.right() - 54.0, rect.center().y - 11.0),
            Vec2::new(40.0, 22.0),
        );
        let mut t_ui = ui.new_child(egui::UiBuilder::new().max_rect(toggle_rect));
        if glass_toggle(&mut t_ui, &mut config.loudness_normalization).changed() {
            config.save();
            action = Some(SettingsAction::LoudnessNormChanged);
        }
    });

    scroll.add_space(10.0);

    // Row 7: Discord Rich Presence
    draw_setting_card(scroll, w, 92.0, |ui, rect| {
        let p = ui.painter();
        p.text(
            Pos2::new(rect.left() + 14.0, rect.top() + 21.0),
            Align2::LEFT_CENTER,
            "Discord Rich Presence",
            font_semibold(13.0),
            TEXT_PRIMARY,
        );

        // Sleek monochrome status badge
        let (badge_text, dot_col, text_col) = match rpc_status {
            RpcStatus::Connected => (
                if is_de { "Verbunden" } else { "Connected" },
                Color32::from_rgb(132, 232, 168),
                TEXT_PRIMARY,
            ),
            RpcStatus::WaitingForDiscord => (
                if is_de {
                    "Wartet auf Discord"
                } else {
                    "Waiting for Discord"
                },
                Color32::from_rgb(218, 198, 128),
                TEXT_SECONDARY,
            ),
            RpcStatus::NeedsClientId => (
                if is_de {
                    "App-ID benötigt"
                } else {
                    "App ID needed"
                },
                Color32::from_rgb(160, 166, 178),
                TEXT_SECONDARY,
            ),
            RpcStatus::Disabled => (
                if is_de { "Aus" } else { "Off" },
                TEXT_MUTED,
                TEXT_MUTED,
            ),
        };
        let badge_galley = p.layout_no_wrap(badge_text.to_string(), font_semibold(10.5), text_col);
        let badge_w = badge_galley.size().x + 22.0;
        let badge_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 168.0, rect.top() + 11.0),
            Vec2::new(badge_w, 20.0),
        );
        p.rect_filled(badge_rect, CornerRadius::same(10), rgba(255, 255, 255, 10));
        p.rect_stroke(
            badge_rect,
            CornerRadius::same(10),
            Stroke::new(1.0_f32, RIM_SUBTLE),
            StrokeKind::Inside,
        );
        p.circle_filled(
            Pos2::new(badge_rect.left() + 9.0, badge_rect.center().y),
            3.0,
            dot_col,
        );
        p.galley(
            Pos2::new(
                badge_rect.left() + 16.0,
                badge_rect.center().y - badge_galley.size().y * 0.5,
            ),
            badge_galley,
            text_col,
        );

        // Toggle switch
        let toggle_rect = Rect::from_min_size(
            Pos2::new(rect.right() - 54.0, rect.top() + 10.0),
            Vec2::new(40.0, 22.0),
        );
        let mut t_ui = ui.new_child(egui::UiBuilder::new().max_rect(toggle_rect));
        if glass_toggle(&mut t_ui, &mut config.discord_rpc_enabled).changed() {
            config.save();
            action = Some(SettingsAction::DiscordRpcChanged);
        }

        // Compact Discord App-ID input row
        ui.painter().text(
            Pos2::new(rect.left() + 14.0, rect.top() + 63.0),
            Align2::LEFT_CENTER,
            "Discord App-ID",
            font_regular(11.5),
            TEXT_SECONDARY,
        );

        let input_rect = Rect::from_min_max(
            Pos2::new(rect.left() + 116.0, rect.top() + 49.0),
            Pos2::new(rect.right() - 14.0, rect.top() + 77.0),
        );
        let mut id_ui = ui.new_child(egui::UiBuilder::new().max_rect(input_rect));
        let hint = if is_de {
            "Discord Developer App-ID eingeben…"
        } else {
            "Enter Discord Developer App ID…"
        };
        let resp = id_ui.add_sized(
            input_rect.size(),
            egui::TextEdit::singleline(&mut config.discord_client_id)
                .font(font_regular(11.5))
                .hint_text(hint)
                .margin(egui::Margin::symmetric(8, 5)),
        );
        if resp.changed() {
            config.save();
            action = Some(SettingsAction::DiscordRpcChanged);
        }
    });

    scroll.add_space(4.0);
        });

    action
}

fn draw_setting_card(ui: &mut Ui, width: f32, height: f32, add_contents: impl FnOnce(&mut Ui, Rect)) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), GLASS_PILL_IDLE);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, RIM_SUBTLE),
        StrokeKind::Inside,
    );
    add_contents(ui, rect);
}
