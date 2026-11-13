//! State carried by one material point.

use glam::{DMat2, DVec2};

/// Deviatoric Cauchy stress in plane strain, in Pa. The out-of-plane normal
/// component follows from the zero trace.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Deviator {
    /// Normal component along `x`.
    pub xx: f64,
    /// Normal component along `y`.
    pub yy: f64,
    /// In-plane shear component.
    pub xy: f64,
}

impl Deviator {
    /// Normal component along `z`.
    #[must_use]
    pub fn zz(&self) -> f64 {
        -(self.xx + self.yy)
    }

    /// Von Mises equivalent stress, `sqrt(3/2 · s:s)`.
    #[must_use]
    pub fn equivalent_stress(&self) -> f64 {
        let squares = self.xx.powi(2) + self.yy.powi(2) + self.zz().powi(2);
        (1.5 * (squares + 2.0 * self.xy.powi(2))).sqrt()
    }

    /// The deviator multiplied by `factor`.
    #[must_use]
    pub fn scaled(&self, factor: f64) -> Self {
        Self {
            xx: self.xx * factor,
            yy: self.yy * factor,
            xy: self.xy * factor,
        }
    }
}

/// One material point. Extensive quantities are per metre of depth in a
/// planar model and per radian in an axisymmetric one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    /// Position, in m.
    pub position: DVec2,
    /// Velocity, in m/s.
    pub velocity: DVec2,
    /// Velocity gradient `∂v_i/∂x_j`, in 1/s.
    pub velocity_gradient: DMat2,
    /// Mass, in kg.
    pub mass: f64,
    /// Current volume, in m³.
    pub volume: f64,
    /// Body the particle belongs to.
    pub body: usize,
    /// Index of the particle's material in [`crate::Model::materials`].
    pub material: usize,
    /// Deviatoric stress, in Pa.
    pub deviator: Deviator,
    /// Pressure from the equation of state, in Pa, positive in compression.
    pub pressure: f64,
    /// Artificial bulk-viscosity pressure, in Pa.
    pub viscous_pressure: f64,
    /// Specific internal energy above the reference state, in J/kg.
    pub specific_energy: f64,
    /// Accumulated equivalent plastic strain.
    pub plastic_strain: f64,
    /// Equivalent plastic strain rate over the last step, in 1/s.
    pub plastic_strain_rate: f64,
    /// Temperature, in K.
    pub temperature: f64,
    /// Accumulated ductile damage in `[0, 1]`. At one the material has
    /// failed: it keeps no shear strength and carries no tension.
    pub damage: f64,
    /// Longitudinal elastic wave speed, in m/s; limits the time step.
    pub sound_speed: f64,
}

impl Particle {
    /// Current density, in kg/m³.
    #[must_use]
    pub fn density(&self) -> f64 {
        self.mass / self.volume
    }

    /// Whether the material point has failed.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.damage >= 1.0
    }

    /// Normal Cauchy stress along `x`, in Pa, positive in tension.
    /// The artificial viscosity is not included.
    #[must_use]
    pub fn stress_xx(&self) -> f64 {
        self.deviator.xx - self.pressure
    }
}

#[cfg(test)]
mod tests {
    use super::Deviator;

    #[test]
    fn uniaxial_stress_deviator_has_the_axial_stress_as_equivalent() {
        // Uniaxial stress σ along x: s = (2σ/3, −σ/3, −σ/3).
        let stress = 300.0e6;
        let deviator = Deviator {
            xx: 2.0 * stress / 3.0,
            yy: -stress / 3.0,
            xy: 0.0,
        };
        assert!((deviator.zz() + stress / 3.0).abs() < 1e-3);
        assert!((deviator.equivalent_stress() - stress).abs() < 1e-3);
    }

    #[test]
    fn pure_shear_equivalent_is_root_three_times_shear() {
        let deviator = Deviator {
            xx: 0.0,
            yy: 0.0,
            xy: 100.0e6,
        };
        assert!((deviator.equivalent_stress() - 3.0_f64.sqrt() * 100.0e6).abs() < 1e-3);
    }

    #[test]
    fn scaling_scales_the_equivalent_stress() {
        let deviator = Deviator {
            xx: 1.0e8,
            yy: -4.0e7,
            xy: 2.5e7,
        };
        let ratio = deviator.scaled(0.25).equivalent_stress() / deviator.equivalent_stress();
        assert!((ratio - 0.25).abs() < 1e-12);
    }
}
