//! Editor of the materials defined by the scenario.

use eframe::egui;
use rap_domain::MaterialId;
use rap_materials::{Damage, Eos, Material, Strength, library};

use crate::widgets::{row, scaled};

/// Identifiers of every material a scenario can use: its own first, then
/// the built-in ones it does not replace.
pub(crate) fn available(custom: &[Material]) -> Vec<MaterialId> {
    let mut ids: Vec<MaterialId> = custom.iter().map(|material| material.id.clone()).collect();
    for material in library::builtin() {
        if !ids.contains(&material.id) {
            ids.push(material.id);
        }
    }
    ids
}

/// An identifier based on `base` that none of `existing` uses.
pub(crate) fn unique_id(base: &str, existing: &[MaterialId]) -> MaterialId {
    let taken = |candidate: &str| existing.iter().any(|id| id.as_str() == candidate);
    let first = format!("{base}-custom");
    if !taken(&first) {
        return MaterialId::new(first);
    }
    (2..u32::MAX)
        .map(|number| format!("{base}-custom-{number}"))
        .find(|candidate| !taken(candidate))
        .map_or_else(|| MaterialId::new(first), MaterialId::new)
}

/// A copy of `material` under a new identifier, ready to be edited.
pub(crate) fn duplicate(material: &Material, existing: &[MaterialId]) -> Material {
    let mut copy = material.clone();
    copy.id = unique_id(material.id.as_str(), existing);
    copy.name = format!("{} (custom)", material.name);
    copy
}

fn equation_of_state(ui: &mut egui::Ui, material: &mut Material) -> bool {
    let mut changed = false;
    if let Eos::MieGruneisen(eos) = &mut material.eos {
        changed |= row(ui, "Density", |ui| {
            scaled(
                ui,
                &mut eos.reference_density,
                1.0,
                100.0..=30_000.0,
                " kg/m³",
            )
        });
        changed |= row(ui, "Bulk sound speed", |ui| {
            scaled(ui, &mut eos.bulk_sound_speed, 1.0, 100.0..=20_000.0, " m/s")
        });
        changed |= row(ui, "Hugoniot slope", |ui| {
            scaled(ui, &mut eos.hugoniot_slope, 1.0, 0.0..=5.0, "")
        });
        changed |= row(ui, "Grüneisen gamma", |ui| {
            scaled(ui, &mut eos.gruneisen_gamma, 1.0, 0.0..=5.0, "")
        });
    }
    changed
}

fn strength(ui: &mut egui::Ui, material: &mut Material) -> bool {
    let mut changed = false;
    if let Strength::JohnsonCook(strength) = &mut material.strength {
        changed |= row(ui, "Shear modulus", |ui| {
            scaled(
                ui,
                &mut strength.shear_modulus,
                1.0e-9,
                0.1..=1000.0,
                " GPa",
            )
        });
        changed |= row(ui, "Yield stress A", |ui| {
            scaled(
                ui,
                &mut strength.yield_stress,
                1.0e-6,
                1.0..=10_000.0,
                " MPa",
            )
        });
        changed |= row(ui, "Hardening B", |ui| {
            scaled(
                ui,
                &mut strength.hardening_modulus,
                1.0e-6,
                0.0..=10_000.0,
                " MPa",
            )
        });
        changed |= row(ui, "Hardening exponent n", |ui| {
            scaled(ui, &mut strength.hardening_exponent, 1.0, 0.0..=2.0, "")
        });
        changed |= row(ui, "Rate coefficient C", |ui| {
            scaled(ui, &mut strength.rate_coefficient, 1.0, 0.0..=1.0, "")
        });
        changed |= row(ui, "Thermal exponent m", |ui| {
            scaled(ui, &mut strength.thermal_exponent, 1.0, 0.1..=5.0, "")
        });
        let mut melting = strength.reference.melting_temperature;
        if row(ui, "Melting temperature", |ui| {
            scaled(ui, &mut melting, 1.0, 300.0..=6000.0, " K")
        }) {
            strength.reference.melting_temperature = melting;
            if let Damage::JohnsonCook(damage) = &mut material.damage {
                damage.reference.melting_temperature = melting;
            }
            changed = true;
        }
    }
    changed
}

fn damage(ui: &mut egui::Ui, material: &mut Material) -> bool {
    let mut changed = false;
    if let Damage::JohnsonCook(damage) = &mut material.damage {
        for (label, constant) in [
            ("Damage D1", &mut damage.d1),
            ("Damage D2", &mut damage.d2),
            ("Damage D3", &mut damage.d3),
            ("Damage D4", &mut damage.d4),
            ("Damage D5", &mut damage.d5),
        ] {
            changed |= row(ui, label, |ui| scaled(ui, constant, 1.0, -10.0..=10.0, ""));
        }
    }
    changed
}

/// Properties of the models used for ceramics.
fn ceramic(ui: &mut egui::Ui, material: &mut Material) -> bool {
    let mut changed = false;
    if let Eos::Polynomial(eos) = &mut material.eos {
        changed |= row(ui, "Density", |ui| {
            scaled(
                ui,
                &mut eos.reference_density,
                1.0,
                100.0..=30_000.0,
                " kg/m³",
            )
        });
        for (label, coefficient) in [
            ("Bulk modulus K1", &mut eos.k1),
            ("K2", &mut eos.k2),
            ("K3", &mut eos.k3),
        ] {
            changed |= row(ui, label, |ui| {
                scaled(ui, coefficient, 1.0e-9, -1000.0..=1000.0, " GPa")
            });
        }
    }
    if let Strength::JohnsonHolmquist(strength) = &mut material.strength {
        changed |= row(ui, "Shear modulus", |ui| {
            scaled(
                ui,
                &mut strength.shear_modulus,
                1.0e-9,
                0.1..=1000.0,
                " GPa",
            )
        });
        changed |= row(ui, "Stress at HEL", |ui| {
            scaled(ui, &mut strength.hel_stress, 1.0e-9, 0.01..=100.0, " GPa")
        });
        let (mut hel_pressure, mut tension) = (strength.hel_pressure, strength.tensile_strength);
        let edited = row(ui, "Pressure at HEL", |ui| {
            scaled(ui, &mut hel_pressure, 1.0e-9, 0.01..=100.0, " GPa")
        }) | row(ui, "Tensile strength T", |ui| {
            scaled(ui, &mut tension, 1.0e-9, 0.0..=10.0, " GPa")
        });
        if edited {
            // The damage model uses the same two values.
            strength.hel_pressure = hel_pressure;
            strength.tensile_strength = tension;
            if let Damage::JohnsonHolmquist(damage) = &mut material.damage {
                damage.hel_pressure = hel_pressure;
                damage.tensile_strength = tension;
            }
            changed = true;
        }
        for (label, constant, upper) in [
            ("Intact A", &mut strength.intact_coefficient, 5.0),
            ("Intact N", &mut strength.intact_exponent, 5.0),
            ("Fractured B", &mut strength.fractured_coefficient, 5.0),
            ("Fractured M", &mut strength.fractured_exponent, 5.0),
            ("Fractured limit", &mut strength.fractured_limit, 10.0),
            ("Rate coefficient C", &mut strength.rate_coefficient, 1.0),
        ] {
            changed |= row(ui, label, |ui| scaled(ui, constant, 1.0, 0.0..=upper, ""));
        }
    }
    if let Damage::JohnsonHolmquist(damage) = &mut material.damage {
        changed |= row(ui, "Damage D1", |ui| {
            scaled(ui, &mut damage.d1, 1.0, 1.0e-6..=10.0, "")
        });
        changed |= row(ui, "Damage D2", |ui| {
            scaled(ui, &mut damage.d2, 1.0, 0.0..=10.0, "")
        });
    }
    changed
}

fn properties(ui: &mut egui::Ui, material: &mut Material) -> bool {
    let mut changed = false;
    changed |= row(ui, "Name", |ui| {
        ui.text_edit_singleline(&mut material.name).changed()
    });
    changed |= equation_of_state(ui, material);
    changed |= strength(ui, material);
    changed |= damage(ui, material);
    changed |= ceramic(ui, material);
    changed |= row(ui, "Spall strength", |ui| {
        scaled(
            ui,
            &mut material.spall_strength,
            1.0e-9,
            0.01..=100.0,
            " GPa",
        )
    });
    changed |= row(ui, "Tensile strength", |ui| {
        ui.horizontal(|ui| {
            let mut limited = material.tensile_strength.is_some();
            let mut edited = ui.checkbox(&mut limited, "").changed();
            if edited {
                material.tensile_strength = limited.then_some(2.0e9);
            }
            if let Some(limit) = &mut material.tensile_strength {
                edited |= scaled(ui, limit, 1.0e-9, 0.01..=100.0, " GPa");
            }
            edited
        })
        .inner
    });
    changed |= row(ui, "Specific heat", |ui| {
        scaled(
            ui,
            &mut material.specific_heat,
            1.0,
            10.0..=5000.0,
            " J/(kg·K)",
        )
    });
    changed
}

/// Editor of the scenario's own materials. Built-in materials are read-only
/// and can be copied to start a custom one. Returns whether anything
/// changed.
pub(crate) fn editor(ui: &mut egui::Ui, custom: &mut Vec<Material>) -> bool {
    let mut changed = false;
    ui.strong("Built-in");
    for material in library::builtin() {
        ui.horizontal(|ui| {
            ui.label(material.id.as_str());
            if ui.small_button("Copy to edit").clicked() {
                let copy = duplicate(&material, &available(custom));
                custom.push(copy);
                changed = true;
            }
        });
    }
    ui.add_space(4.0);
    ui.strong("Defined by this scenario");
    if custom.is_empty() {
        ui.weak("None. Copy a built-in material to adjust its properties.");
    }
    let mut removed = None;
    for (index, material) in custom.iter_mut().enumerate() {
        egui::CollapsingHeader::new(material.id.as_str())
            .id_salt(("custom material", index))
            .show(ui, |ui| {
                egui::Grid::new(("material properties", index))
                    .num_columns(2)
                    .show(ui, |ui| {
                        changed |= properties(ui, material);
                    });
                if ui.button("✖ Remove").clicked() {
                    removed = Some(index);
                }
            });
    }
    if let Some(index) = removed {
        custom.remove(index);
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use rap_domain::MaterialId;
    use rap_materials::library;

    use super::{available, duplicate, unique_id};

    #[test]
    fn custom_materials_come_first_and_hide_built_in_ones_they_replace() {
        let mut replacement = library::steel_4340();
        replacement.name = "Softer steel".to_owned();
        let mut extra = library::steel_4340();
        extra.id = MaterialId::new("aluminium");
        let ids = available(&[extra, replacement]);
        let names: Vec<_> = ids.iter().map(MaterialId::as_str).collect();
        assert_eq!(
            names,
            [
                "aluminium",
                "steel-4340",
                "cast-armor-steel",
                "tungsten-heavy-alloy",
                "alumina-99.5",
                "shell-steel",
                "tnt-inert",
                "tnt"
            ]
        );
    }

    #[test]
    fn identifiers_of_copies_are_unique() {
        let existing = [
            MaterialId::new("steel-4340"),
            MaterialId::new("steel-4340-custom"),
        ];
        assert_eq!(
            unique_id("steel-4340", &existing[..1]).as_str(),
            "steel-4340-custom"
        );
        assert_eq!(
            unique_id("steel-4340", &existing).as_str(),
            "steel-4340-custom-2"
        );
    }

    #[test]
    fn duplicate_is_a_valid_material_under_a_new_identifier() {
        let steel = library::steel_4340();
        let copy = duplicate(&steel, &available(&[]));
        assert_ne!(copy.id, steel.id);
        assert_eq!(copy.validate(), Ok(()));
        assert_eq!(copy.eos, steel.eos);
    }

    #[test]
    fn every_built_in_material_can_be_duplicated() {
        for material in library::builtin() {
            let copy = duplicate(&material, &available(&[]));
            assert_eq!(copy.validate(), Ok(()), "{}", material.id);
        }
    }
}
