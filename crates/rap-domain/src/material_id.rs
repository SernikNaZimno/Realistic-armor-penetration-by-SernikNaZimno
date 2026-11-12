//! Identifier linking domain objects to entries of a material library.

use std::fmt;

/// Identifier of the built-in 4340 steel, also used as a rolled-armor surrogate.
pub const STEEL_4340: &str = "steel-4340";
/// Identifier of the built-in tungsten heavy alloy.
pub const TUNGSTEN_HEAVY_ALLOY: &str = "tungsten-heavy-alloy";

/// Key of a material in a material library.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
