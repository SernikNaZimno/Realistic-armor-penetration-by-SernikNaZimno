//! Identifier linking domain objects to entries of a material library.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Identifier of the built-in 4340 steel, also used as a rolled-armor surrogate.
pub const STEEL_4340: &str = "steel-4340";
/// Identifier of the built-in tungsten heavy alloy.
pub const TUNGSTEN_HEAVY_ALLOY: &str = "tungsten-heavy-alloy";

/// Identifier of the built-in 99.5 % alumina ceramic.
pub const ALUMINA: &str = "alumina-99.5";
/// Identifier of the built-in hardened steel of armor-piercing shells.
pub const SHELL_STEEL: &str = "shell-steel";
/// Identifier of the built-in inert stand-in for a TNT bursting charge.
pub const TNT_INERT: &str = "tnt-inert";

/// Key of a material in a material library.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MaterialId(String);

impl MaterialId {
    /// Creates an identifier from any string-like value.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for MaterialId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl fmt::Display for MaterialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
