//! Numerical settings chosen by the user for one simulation.

use serde::{Deserialize, Serialize};

use crate::validation::{self, InvalidParameter};

/// How finely and how long a scenario is simulated. Every field has a
/// default, so a scenario file may omit any of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SimulationSettings {
    /// Number of grid cells across the projectile radius. Sets the cell
    /// size; the cost grows with its cube.
    pub cells_per_radius: f64,
    /// Number of particles along each edge of a cell.
    pub particles_per_cell_edge: u32,
    /// Half-width of the armor, measured from the shot line, in projectile
    /// radii.
    pub armor_extent_radii: f64,
    /// Coulomb friction coefficient between projectile and armor.
    pub friction: f64,
    /// Simulated time, in seconds. If absent, twice the time the projectile
    /// needs to travel its own length plus the armor along the shot line.
    pub end_time: Option<f64>,
    /// Number of snapshots written over the simulated time.
    pub snapshots: u32,
}

impl Default for SimulationSettings {
    fn default() -> Self {
        Self {
            cells_per_radius: 4.0,
            particles_per_cell_edge: 2,
            armor_extent_radii: 10.0,
            friction: 0.0,
            end_time: None,
            snapshots: 20,
        }
    }
}

impl SimulationSettings {
    /// Checks the settings.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid setting.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        validation::within(
            "cells_per_radius",
            self.cells_per_radius,
            &(1.0..=64.0),
            "between 1 and 64",
        )?;
        validation::within(
            "particles_per_cell_edge",
            f64::from(self.particles_per_cell_edge),
            &(1.0..=4.0),
            "between 1 and 4",
        )?;
        validation::within(
            "armor_extent_radii",
            self.armor_extent_radii,
            &(2.0..=100.0),
            "between 2 and 100",
        )?;
        validation::non_negative("friction", self.friction)?;
        if let Some(end_time) = self.end_time {
            validation::positive("end_time", end_time)?;
        }
        validation::within(
            "snapshots",
            f64::from(self.snapshots),
            &(1.0..=10_000.0),
            "between 1 and 10000",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::SimulationSettings;

    #[test]
    fn defaults_are_valid() {
        assert_eq!(SimulationSettings::default().validate(), Ok(()));
    }

    #[test]
    fn out_of_range_settings_are_rejected() {
        let coarse = SimulationSettings {
            cells_per_radius: 0.5,
            ..SimulationSettings::default()
        };
        assert_eq!(coarse.validate().unwrap_err().field, "cells_per_radius");
        let instant = SimulationSettings {
            end_time: Some(0.0),
            ..SimulationSettings::default()
        };
        assert_eq!(instant.validate().unwrap_err().field, "end_time");
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let settings: SimulationSettings =
            serde_json::from_str(r#"{ "cells_per_radius": 6.0 }"#).expect("valid settings");
        assert_eq!(
            settings,
            SimulationSettings {
                cells_per_radius: 6.0,
                ..SimulationSettings::default()
            }
        );
        assert!(serde_json::from_str::<SimulationSettings>(r#"{ "cell_size": 1.0 }"#).is_err());
    }
}
