//! Detonation of an explosive at a set time.
//!
//! The whole charge reacts in the step in which the time is reached: each
//! of its points becomes a point of the products, gains the energy of the
//! detonation at the volume it has, and from then on is failed material,
//! which carries pressure but neither tension nor shear. This is a
//! constant-volume explosion. The detonation wave that in reality crosses
//! the charge is left out; it crosses a charge of 100 mm in about 15 µs,
//! a few tens of steps.

/// When an explosive goes off and what it turns into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detonation {
    /// Time at which the charge reacts, in s.
    pub time: f64,
    /// Index of the material of the charge.
    pub material: usize,
    /// Index of the material of its products.
    pub products: usize,
    /// Energy released, in J/kg.
    pub energy: f64,
}

impl Detonation {
    /// Whether a point of `material` reacts at `time`.
    #[must_use]
    pub fn reaches(&self, time: f64, material: usize) -> bool {
        material == self.material && time >= self.time
    }
}

#[cfg(test)]
mod tests {
    use super::Detonation;

    #[test]
    fn only_the_charge_reacts_and_only_from_the_set_time() {
        let detonation = Detonation {
            time: 1.0e-3,
            material: 1,
            products: 3,
            energy: 4.3e6,
        };
        assert!(!detonation.reaches(0.999e-3, 1));
        assert!(detonation.reaches(1.0e-3, 1));
        assert!(detonation.reaches(2.0e-3, 1));
        assert!(!detonation.reaches(2.0e-3, 0));
        assert!(!detonation.reaches(2.0e-3, 3));
    }
}
