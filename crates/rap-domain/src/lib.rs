//! Core domain types: projectile, armor, layer, scenario and physical units.
//!
//! All quantities are stored in SI units (`m`, `kg`, `s`, `K`, `rad`) as `f64`.
//! In serialized form angles are given in degrees, in fields ending in `_deg`.
//! Types expose public fields and a `validate` method; callers validate at
//! system boundaries (file loading, user input) before handing data to a solver.

pub mod armor;
mod degrees;
pub mod material_id;
pub mod projectile;
pub mod scenario;
pub mod settings;
pub mod validation;

pub use armor::{Armor, ArmorLayer};
pub use material_id::MaterialId;
pub use projectile::{ApbcProjectile, ApfsdsProjectile, BallisticCap, NoseShape, Projectile};
pub use scenario::Scenario;
pub use settings::SimulationSettings;
pub use validation::{DomainError, InvalidParameter};

/// Human-readable application name shown by every front end.
pub const APP_NAME: &str = "Realistic armor penetration by SernikNaZimno";
