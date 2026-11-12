//! Constitutive models: strength, equation of state and damage.
//!
//! Behaviour is defined by three traits — [`EquationOfState`],
//! [`StrengthModel`] and [`DamageModel`] — so solvers stay generic. The enums
//! [`Eos`], [`Strength`] and [`Damage`] collect the built-in models, giving
//! static dispatch in particle loops and a closed set that can be stored in
//! files. Adding a model means adding a struct, a trait implementation and
//! one enum variant.
//!
//! All quantities are in SI units.

pub mod eos;
pub mod johnson_cook;
pub mod library;
pub mod material;

pub use eos::{Eos, MieGruneisen};
pub use johnson_cook::{JohnsonCookDamage, JohnsonCookStrength, RateTemperatureReference};
pub use material::{Damage, Material, Strength};

/// Pressure response of a material to compression and heating.
pub trait EquationOfState {
    /// Density at zero pressure and zero specific internal energy, in kg/m³.
    fn reference_density(&self) -> f64;

    /// Pressure in Pa, positive in compression, for a density in kg/m³ and a
    /// specific internal energy in J/kg measured from the reference state.
    fn pressure(&self, density: f64, specific_energy: f64) -> f64;

    /// Isentropic bulk sound speed in m/s at the given state.
    fn sound_speed(&self, density: f64, specific_energy: f64) -> f64;
}

/// Plastic state of a material point, as seen by strength and damage models.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlasticState {
    /// Accumulated equivalent plastic strain.
    pub strain: f64,
    /// Equivalent plastic strain rate, in 1/s.
    pub strain_rate: f64,
    /// Temperature, in K.
    pub temperature: f64,
}

/// Resistance of a material to plastic flow.
pub trait StrengthModel {
    /// Elastic shear modulus, in Pa.
    fn shear_modulus(&self) -> f64;

    /// Current flow (yield) stress, in Pa.
    fn flow_stress(&self, state: PlasticState) -> f64;
}

/// Ductile failure criterion based on accumulated plastic strain.
pub trait DamageModel {
    /// Equivalent plastic strain at failure for a constant stress
    /// triaxiality (mean stress divided by equivalent stress, positive in
    /// tension) and plastic state. Always positive.
    fn failure_strain(&self, triaxiality: f64, state: PlasticState) -> f64;

    /// Damage added by a plastic strain increment; the material point has
    /// failed once the accumulated damage reaches one.
    fn damage_increment(
        &self,
        strain_increment: f64,
        triaxiality: f64,
        state: PlasticState,
    ) -> f64 {
        strain_increment / self.failure_strain(triaxiality, state)
    }
}
