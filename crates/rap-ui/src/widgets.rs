//! Small input widgets shared by the editors. Each returns whether the
//! user changed the value.

use std::ops::RangeInclusive;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Shape, Stroke, Vec2};
use rap_domain::MaterialId;

/// A number shown and edited as `value · scale` with a unit suffix.
pub(crate) fn scaled(
    ui: &mut egui::Ui,
    value: &mut f64,
    scale: f64,
    range: RangeInclusive<f64>,
    suffix: &str,
) -> bool {
    let mut shown = *value * scale;
    let speed = (range.end() - range.start()).abs().min(1.0e3) / 400.0;
    let changed = ui
        .add(
            egui::DragValue::new(&mut shown)
                .speed(speed)
                .range(range)
                .clamp_existing_to_range(false)
                .suffix(suffix),
        )
        .changed();
    if changed {
        *value = shown / scale;
    }
    changed
}

/// A length stored in metres, edited in millimetres.
pub(crate) fn millimetres(ui: &mut egui::Ui, metres: &mut f64, range: RangeInclusive<f64>) -> bool {
    scaled(ui, metres, 1.0e3, range, " mm")
}

/// An angle stored in radians, edited in degrees.
pub(crate) fn degrees(ui: &mut egui::Ui, radians: &mut f64, range: RangeInclusive<f64>) -> bool {
    let mut shown = radians.to_degrees();
    let changed = ui
        .add(
            egui::DragValue::new(&mut shown)
                .speed(0.25)
                .range(range)
                .clamp_existing_to_range(false)
                .suffix("°"),
        )
        .changed();
    if changed {
        *radians = shown.to_radians();
    }
    changed
}

/// A drop-down list of the available materials.
pub(crate) fn material_picker(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    current: &mut MaterialId,
    available: &[MaterialId],
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(current.as_str())
        .show_ui(ui, |ui| {
            for material in available {
                if ui
                    .selectable_label(material == current, material.as_str())
                    .clicked()
                    && material != current
                {
                    *current = material.clone();
                    changed = true;
                }
            }
        });
    changed
}

/// A two-column grid row: a label and one widget.
pub(crate) fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    ui.label(label);
    let changed = add(ui);
    ui.end_row();
    changed
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "chart coordinates fit in f32 comfortably"
)]
fn to_f32(value: f64) -> f32 {
    value as f32
}

/// A minimal line chart of `points` with both axes starting at zero.
pub(crate) fn line_chart(ui: &mut egui::Ui, points: &[[f64; 2]], caption: &str, height: f32) {
    let size = Vec2::new(ui.available_width(), height);
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let frame = response.rect.shrink(4.0);
    let visuals = ui.visuals();
    painter.rect_stroke(
        frame,
        0.0,
        Stroke::new(1.0_f32, visuals.weak_text_color()),
        egui::StrokeKind::Inside,
    );
    let font = FontId::proportional(11.0);
    let text = visuals.text_color();
    painter.text(
        frame.left_top() + Vec2::new(4.0, 2.0),
        Align2::LEFT_TOP,
        caption,
        font.clone(),
        text,
    );

    let highest = points.iter().fold([0.0_f64; 2], |highest, point| {
        [highest[0].max(point[0]), highest[1].max(point[1])]
    });
    if highest[0] <= 0.0 || highest[1] <= 0.0 {
        return;
    }
    let plot = frame.shrink2(Vec2::new(6.0, 16.0));
    let line: Vec<Pos2> = points
        .iter()
        .map(|point| {
            Pos2::new(
                plot.left() + plot.width() * to_f32(point[0] / highest[0]),
                plot.bottom() - plot.height() * to_f32(point[1] / highest[1]),
            )
        })
        .collect();
    painter.add(Shape::line(
        line,
        Stroke::new(1.5_f32, Color32::from_rgb(230, 145, 56)),
    ));
    painter.text(
        frame.right_top() + Vec2::new(-4.0, 2.0),
        Align2::RIGHT_TOP,
        format!("max {:.1}", highest[1]),
        font.clone(),
        text,
    );
    painter.text(
        frame.right_bottom() + Vec2::new(-4.0, -2.0),
        Align2::RIGHT_BOTTOM,
        format!("{:.0}", highest[0]),
        font,
        text,
    );
}
