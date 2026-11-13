//! Serde adapter storing an angle held in radians as degrees.

use serde::{Deserialize, Deserializer, Serializer};

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "the signature is dictated by serde"
)]
pub(crate) fn serialize<S: Serializer>(radians: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_f64(radians.to_degrees())
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    f64::deserialize(deserializer).map(f64::to_radians)
}
