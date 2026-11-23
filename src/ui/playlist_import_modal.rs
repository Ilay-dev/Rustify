use crate::config::AppConfig;
use crate::i18n::Language;
use crate::ui::icons::{draw_icon, Icon};
use crate::ui::theme::{
    draw_liquid_glass_panel, font_regular, font_semibold, frosted_cta_button, glass_icon_button,
    glass_pill_button, rgba, GlassLevel, FROST_CTA_IDLE, FROST_CTA_TEXT, GLASS_PILL_IDLE,
    RIM_BRIGHT, RIM_SUBTLE, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportTab {
    #[default]
    PcFolder,
    EmptyLocal,
}

#[derive(Default)]
pub struct PlaylistImportModalState {
    pub open: bool,
    pub tab: ImportTab,
    pub new_playlist_name: String,
}

impl PlaylistImportModalState {
    pub fn open_from_sidebar(&mut self) {
        self.open = true;
        self.tab = ImportTab::PcFolder;
    }
}

pub enum PlaylistImportAction {
    ImportPcFolder(PathBuf),
    CreateEmptyPlaylist(String),
}

pub fn show_playlist_import_modal(
    ui: &mut Ui,
    screen: Rect,
    config: &AppConfig,
    state: &mut PlaylistImportModalState,
) -> Option<PlaylistImportAction> {
    if !state.open {
        return None;
    }
    let is_de = config.language == Language::De;
    let mut action = None;

    let scrim_resp = ui.interact(screen, ui.id().with("import_modal_scrim"), Sense::click());
    ui.painter().rect_filled(
        screen,
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(0, 0, 0, 180),
    );

    let card_w = 480.0f32.min(screen.width() - 40.0);
    let card_h = 330.0f32.min(screen.height() - 40.0);
    let card_rect = Rect::from_center_size(screen.center(), Vec2::new(card_w, card_h));

    if scrim_resp.clicked()
        && !card_rect.contains(scrim_resp.interact_pointer_pos().unwrap_or(Pos2::ZERO))
    {
        state.open = false;
        return None;
    }

    draw_liquid_glass_panel(ui.painter(), card_rect, 16, GlassLevel::Modal);

    let inner = card_rect.shrink2(Vec2::new(24.0, 20.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );

    // Header
    child.horizontal(|ui| {
        ui.label(
            egui::RichText::new(if is_de {
                "Playlist erstellen & importieren"
            } else {
                "Create & Import Playlist"
            })
            .font(font_semibold(17.0))
            .color(TEXT_PRIMARY),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if glass_icon_button(ui, Icon::Close, 26.0, 11.0, false, false).clicked() {
                state.open = false;
            }
        });
    });

    child.add_space(12.0);

    // Uniform 2-Tab Segmented Switcher: [ Ordner vom PC | Neue Playlist ]
    let tabs = [
        (
            ImportTab::PcFolder,
            if is_de { "Ordner vom PC" } else { "PC Folder" },
        ),
        (
            ImportTab::EmptyLocal,
            if is_de { "Neue Playlist" } else { "New Playlist" },
        ),
    ];
    let tab_bar_h = 34.0;
    let (tab_outer, _) =
        child.allocate_exact_size(Vec2::new(inner.width(), tab_bar_h), Sense::hover());
    child
        .painter()
        .rect_filled(tab_outer, CornerRadius::same(8), GLASS_PILL_IDLE);
    child.painter().rect_stroke(
        tab_outer,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, RIM_SUBTLE),
        StrokeKind::Inside,
    );
    let seg_w = (tab_outer.width() - 6.0) / 2.0;
    for (i, (tab, lbl)) in tabs.iter().enumerate() {
        let seg_r = Rect::from_min_size(
            Pos2::new(
                tab_outer.left() + 3.0 + i as f32 * seg_w,
                tab_outer.top() + 3.0,
            ),
            Vec2::new(seg_w, tab_bar_h - 6.0),
        );
        let resp = child
            .interact(seg_r, child.id().with(("imp_tab", i)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let active = state.tab == *tab;
        if active {
            child
                .painter()
                .rect_filled(seg_r, CornerRadius::same(6), FROST_CTA_IDLE);
        } else if resp.hovered() {
            child
                .painter()
                .rect_filled(seg_r, CornerRadius::same(6), rgba(255, 255, 255, 14));
        }
        child.painter().text(
            seg_r.center(),
            Align2::CENTER_CENTER,
            *lbl,
            font_semibold(12.0),
            if active {
                FROST_CTA_TEXT
            } else {
                TEXT_SECONDARY
            },
        );
        if resp.clicked() {
            state.tab = *tab;
        }
    }

    child.add_space(16.0);

    match state.tab {
        ImportTab::PcFolder => {
            let box_r = Rect::from_min_size(child.cursor().min, Vec2::new(inner.width(), 150.0));
            child.allocate_exact_size(box_r.size(), Sense::hover());
            child
                .painter()
                .rect_filled(box_r, CornerRadius::same(8), rgba(255, 255, 255, 6));
            child.painter().rect_stroke(
                box_r,
                CornerRadius::same(8),
                Stroke::new(1.0_f32, RIM_SUBTLE),
                StrokeKind::Inside,
            );
            draw_icon(
                child.painter(),
                Pos2::new(box_r.center().x, box_r.top() + 32.0),
                24.0,
                Icon::Folder,
                TEXT_PRIMARY,
            );
            child.painter().text(
                Pos2::new(box_r.center().x, box_r.top() + 64.0),
                Align2::CENTER_CENTER,
                if is_de {
                    "Lokalen Musikordner vom PC als Playlist importieren"
                } else {
                    "Import a local music folder from your PC as a playlist"
                },
                font_semibold(13.0),
                TEXT_PRIMARY,
            );
            child.painter().text(
                Pos2::new(box_r.center().x, box_r.top() + 85.0),
                Align2::CENTER_CENTER,
                if is_de {
                    "Alle MP3-, WAV-, FLAC-, OGG- und M4A-Dateien werden sauber kopiert."
                } else {
                    "All MP3, WAV, FLAC, OGG, and M4A files will be cleanly copied."
                },
                font_regular(11.5),
                TEXT_SECONDARY,
            );

            let btn_area = Rect::from_center_size(
                Pos2::new(box_r.center().x, box_r.bottom() - 28.0),
                Vec2::new(210.0, 34.0),
            );
            let mut b_ui = child.new_child(
                egui::UiBuilder::new()
                    .max_rect(btn_area)
                    .layout(egui::Layout::centered_and_justified(
                        egui::Direction::LeftToRight,
                    )),
            );
            if frosted_cta_button(
                &mut b_ui,
                Some(Icon::Folder),
                if is_de {
                    "Ordner auswählen"
                } else {
                    "Choose Folder"
                },
                34.0,
            )
            .clicked()
            {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    action = Some(PlaylistImportAction::ImportPcFolder(folder));
                    state.open = false;
                }
            }
        }
        ImportTab::EmptyLocal => {
            child.label(
                egui::RichText::new(if is_de {
                    "NAME DER NEUEN PLAYLIST"
                } else {
                    "NEW PLAYLIST NAME"
                })
                .font(font_semibold(10.0))
                .color(TEXT_MUTED),
            );
            child.add_space(6.0);
            let (box_r, _) =
                child.allocate_exact_size(Vec2::new(inner.width(), 36.0), Sense::hover());
            child
                .painter()
                .rect_filled(box_r, CornerRadius::same(8), GLASS_PILL_IDLE);
            child.painter().rect_stroke(
                box_r,
                CornerRadius::same(8),
                Stroke::new(1.0_f32, RIM_BRIGHT),
                StrokeKind::Inside,
            );
            let mut inp_ui = child
                .new_child(egui::UiBuilder::new().max_rect(box_r.shrink2(Vec2::new(10.0, 7.0))));
            let te = inp_ui.add(
                egui::TextEdit::singleline(&mut state.new_playlist_name)
                    .hint_text(
                        egui::RichText::new(if is_de {
                            "z. B. Gym, Chill, Autofahrt..."
                        } else {
                            "e.g. Gym, Chill, Roadtrip..."
                        })
                        .color(TEXT_MUTED),
                    )
                    .frame(false)
                    .desired_width(f32::INFINITY)
                    .font(font_regular(13.0)),
            );
            if te.lost_focus()
                && child.input(|i| i.key_pressed(egui::Key::Enter))
                && !state.new_playlist_name.trim().is_empty()
            {
                action = Some(PlaylistImportAction::CreateEmptyPlaylist(
                    state.new_playlist_name.trim().to_string(),
                ));
                state.new_playlist_name.clear();
                state.open = false;
            }

            child.add_space(10.0);
            child.label(
                egui::RichText::new(if is_de {
                    "Du kannst Audiodateien anschließend direkt per Drag & Drop in die Playlist ziehen."
                } else {
                    "You can drag & drop audio files directly into the playlist right after creating it."
                })
                .font(font_regular(11.5))
                .color(TEXT_SECONDARY),
            );
        }
    }

    // Bottom Action Bar
    let footer = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - 34.0), inner.max);
    let mut f_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(footer)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );

    if state.tab == ImportTab::EmptyLocal {
        let can_create = !state.new_playlist_name.trim().is_empty();
        f_ui.add_enabled_ui(can_create, |ui| {
            if frosted_cta_button(
                ui,
                None,
                if is_de { "Erstellen" } else { "Create" },
                34.0,
            )
            .clicked()
            {
                action = Some(PlaylistImportAction::CreateEmptyPlaylist(
                    state.new_playlist_name.trim().to_string(),
                ));
                state.new_playlist_name.clear();
                state.open = false;
            }
        });
        f_ui.add_space(10.0);
    }

    if glass_pill_button(
        &mut f_ui,
        None,
        if is_de { "Schließen" } else { "Close" },
        34.0,
        false,
    )
    .clicked()
    {
        state.open = false;
    }

    action
}
