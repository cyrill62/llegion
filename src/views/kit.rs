//! Shared building blocks: boxed panels, stat tiles, badges and form rows.

use eframe::egui::{self, Color32, RichText, Stroke};

pub fn card_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(36, 38, 43)
    } else {
        Color32::WHITE
    }
}

/// Background of the boxes nested inside a card (one per setting).
fn row_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(45, 48, 55)
    } else {
        Color32::from_rgb(245, 246, 249)
    }
}

fn card_stroke(ui: &egui::Ui) -> Stroke {
    let color = if ui.visuals().dark_mode {
        Color32::from_rgb(54, 57, 64)
    } else {
        Color32::from_rgb(220, 223, 228)
    };
    Stroke::new(1.0, color)
}

pub fn card_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(card_fill(ui))
        .stroke(card_stroke(ui))
        .corner_radius(10.0)
        .inner_margin(18.0)
}

/// Full-width box with a title (and optional one-line description).
pub fn panel<R>(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: Option<&str>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let r = card_frame(ui)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(21.0).strong());
            if let Some(sub) = subtitle {
                ui.label(RichText::new(sub).weak());
            }
            ui.add_space(10.0);
            body(ui)
        })
        .inner;
    ui.add_space(14.0);
    r
}

/// Same as [`panel`] with buttons aligned to the right of the title.
pub fn panel_with_actions<R>(
    ui: &mut egui::Ui,
    title: &str,
    actions: impl FnOnce(&mut egui::Ui),
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let r = card_frame(ui)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(21.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
            });
            ui.add_space(10.0);
            body(ui)
        })
        .inner;
    ui.add_space(14.0);
    r
}

/// Box showing one big figure, used in rows built with `ui.columns`.
pub fn tile(ui: &mut egui::Ui, label: &str, value: RichText, sub: Option<String>) {
    card_frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(100.0);
        ui.label(RichText::new(label).weak());
        ui.label(value.size(32.0).strong());
        if let Some(sub) = sub {
            ui.label(RichText::new(sub).weak());
        }
    });
}

/// Small rounded label with a tinted background.
pub fn badge(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).color(color).strong());
        })
        .response
}

/// Horizontal usage bar with text, sized for the larger UI.
pub fn meter(ui: &mut egui::Ui, fraction: f32, text: String, width: f32) {
    let color = if fraction > 0.9 {
        crate::app::RED
    } else if fraction > 0.7 {
        crate::app::AMBER
    } else {
        ui.visuals().selection.bg_fill
    };
    ui.add(
        egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
            .desired_width(width)
            .desired_height(28.0)
            .fill(color)
            .text(text),
    );
}

/// Two-column form: labels on the left, controls on the right.
pub fn form(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([24.0, 14.0])
        .min_col_width(170.0)
        .show(ui, body);
}

/// Compact read-only key/value grid for summary boxes.
pub fn kv_grid(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([20.0, 10.0])
        .min_col_width(110.0)
        .show(ui, body);
}

/// One setting in its own box: title and help on the left, the control on the right.
pub fn setting<R>(
    ui: &mut egui::Ui,
    title: &str,
    help: Option<&str>,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let r = egui::Frame::new()
        .fill(row_fill(ui))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let label_width = (ui.available_width() * 0.4).clamp(160.0, 300.0);
            let control_width = ui.available_width() - label_width - 12.0;
            // A grid centres both cells vertically, whatever their heights.
            egui::Grid::new(ui.next_auto_id())
                .num_columns(2)
                .spacing([12.0, 0.0])
                .min_col_width(label_width)
                .max_col_width(label_width.max(control_width))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(label_width);
                        ui.label(RichText::new(title).strong());
                        if let Some(help) = help {
                            ui.label(RichText::new(help).small().weak());
                        }
                    });
                    ui.allocate_ui_with_layout(
                        egui::vec2(control_width, 0.0),
                        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                        control,
                    )
                    .inner
                })
                .inner
        })
        .inner;
    ui.add_space(8.0);
    r
}

/// On/off switch, with an optional clickable label after it.
pub fn switch(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    let height = 28.0;
    let size = egui::vec2(52.0, height);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if !label.is_empty() {
        response |= ui.add(egui::Label::new(label).sense(egui::Sense::click()));
    }
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, label)
    });
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let visuals = ui.style().interact(&response);
        let off_fill = ui.visuals().widgets.inactive.bg_fill;
        let fill = off_fill.lerp_to_gamma(ui.visuals().selection.bg_fill, t);
        let radius = 0.5 * height;
        ui.painter().rect(
            rect,
            radius,
            fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), radius - 4.0, Color32::WHITE);
    }
    response
}

/// Row of large toggle buttons for picking one value, styled as segmented boxes.
pub fn choice<T: PartialEq + Clone>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) {
    ui.horizontal_wrapped(|ui| {
        for (v, label) in options {
            let selected = *value == *v;
            let mut text = RichText::new(*label);
            if selected {
                text = text.strong();
            }
            let button = egui::Button::selectable(selected, text).min_size(egui::vec2(0.0, 36.0));
            if ui.add(button).clicked() {
                *value = v.clone();
            }
        }
    });
}

/// Form label with an optional explanation underneath.
pub fn field(ui: &mut egui::Ui, label: &str, help: Option<&str>) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).strong());
        if let Some(help) = help {
            ui.label(RichText::new(help).small().weak());
        }
    });
}

/// Monospace box with a copy button.
pub fn code_block(ui: &mut egui::Ui, content: &str) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(8.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(content).monospace());
        });
    ui.add_space(4.0);
    if ui.button(ic!(COPY, "Copy")).clicked() {
        ui.ctx().copy_text(content.to_string());
    }
}

/// Large selectable entry for side navigation lists.
pub fn nav_item(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let mut text = RichText::new(label);
    if selected {
        text = text.strong();
    }
    // The empty right text adds a growing spacer, so the label sits flush left.
    let button = egui::Button::selectable(selected, text)
        .right_text("")
        .min_size(egui::vec2(ui.available_width(), 46.0));
    ui.add(button)
}
