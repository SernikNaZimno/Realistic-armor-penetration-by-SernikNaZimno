//! Editors of the scenario: projectile, armor and simulation settings.
//! Each returns whether the user changed anything.

use eframe::egui;
use rap_domain::{
    ApbcProjectile, ApfsdsProjectile, Armor, ArmorLayer, BallisticCap, MaterialId, NoseShape,
    ProfilePoint, Projectile, Scenario, SimulationSettings,
};

use crate::widgets::{degrees, material_picker, millimetres, row, scaled};

/// Kinds of projectile the editor can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Apbc,
    Apfsds,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Apbc => "APBC — capped full-caliber shot",
            Self::Apfsds => "APFSDS — long rod",
        }
    }
}

/// Kind of `projectile`, if the editor knows it.
pub(crate) fn kind_of(projectile: &Projectile) -> Option<Kind> {
    match projectile {
        Projectile::Apbc(_) => Some(Kind::Apbc),
        Projectile::Apfsds(_) => Some(Kind::Apfsds),
        _ => None,
    }
}

/// A projectile of `kind` with the dimensions, nose and material of
/// `projectile`.
pub(crate) fn converted(projectile: &Projectile, kind: Kind) -> Projectile {
    match kind {
        Kind::Apbc => Projectile::Apbc(ApbcProjectile {
            caliber: projectile.diameter(),
            body_length: projectile.body_length(),
            nose: projectile.nose(),
            cap: projectile.cap().cloned(),
            material: projectile.material().clone(),
        }),
        Kind::Apfsds => Projectile::Apfsds(ApfsdsProjectile {
            rod_diameter: projectile.diameter(),
            rod_length: projectile.body_length(),
            nose: projectile.nose(),
            material: projectile.material().clone(),
        }),
    }
}

/// Shapes of nose offered by the editor, with default parameters.
const NOSES: [(&str, NoseShape); 4] = [
    ("Flat", NoseShape::Flat),
    ("Hemisphere", NoseShape::Hemisphere),
    (
        "Cone",
        NoseShape::Cone {
            half_angle: 0.261_799_387_799_149_4,
        },
    ),
    (
        "Ogive",
        NoseShape::Ogive {
            caliber_radius_head: 1.5,
        },
    ),
];

fn nose_label(nose: NoseShape) -> &'static str {
    match nose {
        NoseShape::Flat => NOSES[0].0,
        NoseShape::Hemisphere => NOSES[1].0,
        NoseShape::Cone { .. } => NOSES[2].0,
        NoseShape::Ogive { .. } => NOSES[3].0,
    }
}

fn nose(ui: &mut egui::Ui, nose: &mut NoseShape) -> bool {
    let mut changed = false;
    changed |= row(ui, "Nose", |ui| {
        let mut picked = false;
        egui::ComboBox::from_id_salt("nose")
            .selected_text(nose_label(*nose))
            .show_ui(ui, |ui| {
                for (label, shape) in NOSES {
                    let selected = nose_label(*nose) == label;
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        *nose = shape;
                        picked = true;
                    }
                }
            });
        picked
    });
    match nose {
        NoseShape::Flat | NoseShape::Hemisphere => {}
        NoseShape::Cone { half_angle } => {
            changed |= row(ui, "Half-angle", |ui| degrees(ui, half_angle, 1.0..=89.0));
        }
        NoseShape::Ogive {
            caliber_radius_head,
        } => {
            changed |= row(ui, "Radius head", |ui| {
                scaled(ui, caliber_radius_head, 1.0, 0.5..=10.0, " cal")
            });
        }
    }
    changed
}

fn cap(ui: &mut egui::Ui, shot: &mut ApbcProjectile, materials: &[MaterialId]) -> bool {
    let mut changed = false;
    let mut fitted = shot.cap.is_some();
    changed |= row(ui, "Ballistic cap", |ui| {
        ui.checkbox(&mut fitted, "fitted").changed()
    });
    if fitted && shot.cap.is_none() {
        shot.cap = Some(BallisticCap {
            length: 1.2 * shot.caliber,
            wall_thickness: 0.03 * shot.caliber,
            material: shot.material.clone(),
        });
    } else if !fitted {
        shot.cap = None;
    }
    if let Some(cap) = &mut shot.cap {
        changed |= row(ui, "Cap length", |ui| {
            millimetres(ui, &mut cap.length, 1.0..=2000.0)
        });
        changed |= row(ui, "Cap wall", |ui| {
            millimetres(ui, &mut cap.wall_thickness, 0.1..=200.0)
        });
        changed |= row(ui, "Cap material", |ui| {
            material_picker(ui, "cap material", &mut cap.material, materials)
        });
    }
    changed
}

/// Editor of the projectile and its impact speed.
pub(crate) fn projectile(
    ui: &mut egui::Ui,
    scenario: &mut Scenario,
    materials: &[MaterialId],
) -> bool {
    let mut changed = false;
    let current = kind_of(&scenario.projectile);
    egui::Grid::new("projectile").num_columns(2).show(ui, |ui| {
        changed |= row(ui, "Type", |ui| {
            let mut picked = false;
            let label = current.map_or("Unknown", Kind::label);
            egui::ComboBox::from_id_salt("kind")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for kind in [Kind::Apbc, Kind::Apfsds] {
                        if ui
                            .selectable_label(current == Some(kind), kind.label())
                            .clicked()
                            && current != Some(kind)
                        {
                            scenario.projectile = converted(&scenario.projectile, kind);
                            picked = true;
                        }
                    }
                });
            picked
        });
        match &mut scenario.projectile {
            Projectile::Apbc(shot) => {
                changed |= row(ui, "Caliber", |ui| {
                    millimetres(ui, &mut shot.caliber, 1.0..=500.0)
                });
                changed |= row(ui, "Body length", |ui| {
                    millimetres(ui, &mut shot.body_length, 1.0..=3000.0)
                });
                changed |= nose(ui, &mut shot.nose);
                changed |= row(ui, "Material", |ui| {
                    material_picker(ui, "shot material", &mut shot.material, materials)
                });
                changed |= cap(ui, shot, materials);
            }
            Projectile::Apfsds(rod) => {
                changed |= row(ui, "Rod diameter", |ui| {
                    millimetres(ui, &mut rod.rod_diameter, 1.0..=200.0)
                });
                changed |= row(ui, "Rod length", |ui| {
                    millimetres(ui, &mut rod.rod_length, 1.0..=3000.0)
                });
                changed |= nose(ui, &mut rod.nose);
                changed |= row(ui, "Material", |ui| {
                    material_picker(ui, "rod material", &mut rod.material, materials)
                });
            }
            _ => {
                ui.label("This projectile type cannot be edited here.");
                ui.end_row();
            }
        }
        changed |= row(ui, "Impact speed", |ui| {
            scaled(ui, &mut scenario.impact_speed, 1.0, 1.0..=10_000.0, " m/s")
        });
    });
    changed
}

/// What the user asked to do with one row of a list.
enum RowAction {
    MoveUp,
    MoveDown,
    Remove,
}

fn row_buttons(ui: &mut egui::Ui, index: usize, count: usize, minimum: usize) -> Option<RowAction> {
    let mut action = None;
    if ui
        .add_enabled(index > 0, egui::Button::new("⏶").small())
        .clicked()
    {
        action = Some(RowAction::MoveUp);
    }
    if ui
        .add_enabled(index + 1 < count, egui::Button::new("⏷").small())
        .clicked()
    {
        action = Some(RowAction::MoveDown);
    }
    if ui
        .add_enabled(count > minimum, egui::Button::new("✖").small())
        .clicked()
    {
        action = Some(RowAction::Remove);
    }
    action
}

fn apply<T>(items: &mut Vec<T>, index: usize, action: &RowAction) {
    match action {
        RowAction::MoveUp => items.swap(index, index - 1),
        RowAction::MoveDown => items.swap(index, index + 1),
        RowAction::Remove => {
            items.remove(index);
        }
    }
}

fn layers(ui: &mut egui::Ui, layers: &mut Vec<ArmorLayer>, materials: &[MaterialId]) -> bool {
    let mut changed = false;
    let mut pending = None;
    let count = layers.len();
    egui::Grid::new("layers")
        .num_columns(5)
        .striped(true)
        .show(ui, |ui| {
            for heading in ["#", "Material", "Thickness", "Gap before", ""] {
                ui.strong(heading);
            }
            ui.end_row();
            for (index, layer) in layers.iter_mut().enumerate() {
                ui.label((index + 1).to_string());
                changed |= material_picker(ui, ("layer", index), &mut layer.material, materials);
                changed |= millimetres(ui, &mut layer.thickness, 0.1..=5000.0);
                if index == 0 {
                    ui.weak("struck face");
                } else {
                    changed |= millimetres(ui, &mut layer.gap_before, 0.0..=5000.0);
                }
                ui.horizontal(|ui| {
                    if let Some(action) = row_buttons(ui, index, count, 1) {
                        pending = Some((index, action));
                    }
                });
                ui.end_row();
            }
        });
    if let Some((index, action)) = pending {
        apply(layers, index, &action);
        // The first plate has nothing in front of it.
        if let Some(first) = layers.first_mut() {
            first.gap_before = 0.0;
        }
        changed = true;
    }
    if ui.button("➕ Add layer").clicked() {
        if let Some(last) = layers.last().cloned() {
            layers.push(last);
            changed = true;
        }
    }
    changed
}

fn face_profile(ui: &mut egui::Ui, profile: &mut Vec<ProfilePoint>) -> bool {
    let mut changed = false;
    let mut pending = None;
    let count = profile.len();
    ui.label(
        "Displacement of the armor face along its normal, by distance from the shot line. \
         Positive values are further from the projectile. No points: flat armor.",
    );
    if count > 0 {
        egui::Grid::new("face profile")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                for heading in ["Distance", "Displacement", ""] {
                    ui.strong(heading);
                }
                ui.end_row();
                for (index, point) in profile.iter_mut().enumerate() {
                    changed |= millimetres(ui, &mut point.lateral, 0.0..=10_000.0);
                    changed |= millimetres(ui, &mut point.offset, -5000.0..=5000.0);
                    if ui.add(egui::Button::new("✖").small()).clicked() {
                        pending = Some(index);
                    }
                    ui.end_row();
                }
            });
    }
    if let Some(index) = pending {
        profile.remove(index);
        changed = true;
    }
    if ui.button("➕ Add point").clicked() {
        profile.push(next_profile_point(profile));
        changed = true;
    }
    changed
}

/// A new profile point beyond the last one, continuing at its displacement.
pub(crate) fn next_profile_point(profile: &[ProfilePoint]) -> ProfilePoint {
    profile.last().map_or(
        ProfilePoint {
            lateral: 0.0,
            offset: 0.0,
        },
        |last| ProfilePoint {
            lateral: last.lateral + 0.02,
            offset: last.offset,
        },
    )
}

/// Editor of the armor: obliquity, layers and face shape.
pub(crate) fn armor(ui: &mut egui::Ui, armor: &mut Armor, materials: &[MaterialId]) -> bool {
    let mut changed = false;
    egui::Grid::new("armor").num_columns(2).show(ui, |ui| {
        changed |= row(ui, "Obliquity", |ui| {
            degrees(ui, &mut armor.obliquity, 0.0..=85.0)
        });
    });
    ui.add_space(4.0);
    ui.strong("Layers");
    changed |= layers(ui, &mut armor.layers, materials);
    ui.add_space(4.0);
    ui.strong("Face shape");
    changed |= face_profile(ui, &mut armor.face_profile);
    changed
}

/// Editor of the numerical settings.
pub(crate) fn settings(ui: &mut egui::Ui, settings: &mut SimulationSettings) -> bool {
    let mut changed = false;
    egui::Grid::new("settings").num_columns(2).show(ui, |ui| {
        changed |= row(ui, "Cells per radius", |ui| {
            scaled(ui, &mut settings.cells_per_radius, 1.0, 1.0..=64.0, "")
        });
        changed |= row(ui, "Particles per cell edge", |ui| {
            ui.add(egui::DragValue::new(&mut settings.particles_per_cell_edge).range(1..=4))
                .changed()
        });
        changed |= row(ui, "Armor half-width", |ui| {
            scaled(
                ui,
                &mut settings.armor_extent_radii,
                1.0,
                2.0..=100.0,
                " radii",
            )
        });
        changed |= row(ui, "Friction", |ui| {
            scaled(ui, &mut settings.friction, 1.0, 0.0..=2.0, "")
        });
        let mut manual = settings.end_time.is_some();
        changed |= row(ui, "Simulated time", |ui| {
            ui.checkbox(&mut manual, "set manually").changed()
        });
        if manual {
            let end_time = settings.end_time.get_or_insert(2.0e-4);
            changed |= row(ui, "", |ui| {
                scaled(ui, end_time, 1.0e6, 1.0..=100_000.0, " µs")
            });
        } else {
            settings.end_time = None;
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use rap_domain::{ApbcProjectile, ApfsdsProjectile, ProfilePoint, Projectile};

    use super::{Kind, RowAction, apply, converted, kind_of, next_profile_point};

    #[test]
    fn conversion_keeps_dimensions_nose_and_material() {
        let rod = Projectile::Apfsds(ApfsdsProjectile::generic_120mm());
        let shot = converted(&rod, Kind::Apbc);
        assert_eq!(kind_of(&shot), Some(Kind::Apbc));
        assert!((shot.diameter() - rod.diameter()).abs() < f64::EPSILON);
        assert!((shot.body_length() - rod.body_length()).abs() < f64::EPSILON);
        assert_eq!(shot.nose(), rod.nose());
        assert_eq!(shot.material(), rod.material());
        assert!(shot.cap().is_none());
        // And back again, unchanged.
        assert_eq!(converted(&shot, Kind::Apfsds), rod);
    }

    #[test]
    fn converting_a_capped_shot_to_itself_keeps_the_cap() {
        let shot = Projectile::Apbc(ApbcProjectile::generic_76mm());
        assert_eq!(converted(&shot, Kind::Apbc), shot);
    }

    #[test]
    fn list_rows_can_be_moved_and_removed() {
        let mut items = vec!['a', 'b', 'c'];
        apply(&mut items, 2, &RowAction::MoveUp);
        assert_eq!(items, ['a', 'c', 'b']);
        apply(&mut items, 0, &RowAction::MoveDown);
        assert_eq!(items, ['c', 'a', 'b']);
        apply(&mut items, 1, &RowAction::Remove);
        assert_eq!(items, ['c', 'b']);
    }

    #[test]
    fn new_profile_points_continue_outwards() {
        let first = next_profile_point(&[]);
        assert_eq!(
            first,
            ProfilePoint {
                lateral: 0.0,
                offset: 0.0
            }
        );
        let second = next_profile_point(&[ProfilePoint {
            lateral: 0.05,
            offset: 0.01,
        }]);
        assert!(second.lateral > 0.05);
        assert!((second.offset - 0.01).abs() < f64::EPSILON);
    }
}
