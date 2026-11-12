//! Parameter validation shared by the domain and the crates built on it.

use std::ops::RangeInclusive;

use thiserror::Error;

/// A numeric parameter that violates its physical requirement.
#[derive(Debug, Clone, PartialEq, Error)]
#[error("parameter `{field}` = {value} is invalid: must be {requirement}")]
pub struct InvalidParameter {
    /// Name of the offending parameter.
    pub field: &'static str,
    /// The rejected value.
    pub value: f64,
    /// Human-readable description of the accepted values.
    pub requirement: &'static str,
}

/// Any reason a domain object can be rejected.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DomainError {
    /// A numeric parameter is out of its valid range.
    #[error(transparent)]
    InvalidParameter(#[from] InvalidParameter),
    /// An armor array was defined without any layer.
    #[error("armor must contain at least one layer")]
    EmptyArmor,
}

fn check(
    field: &'static str,
    value: f64,
    accepted: bool,
    requirement: &'static str,
) -> Result<(), InvalidParameter> {
    if accepted {
        Ok(())
    } else {
        Err(InvalidParameter {
            field,
            value,
            requirement,
        })
    }
}

/// Accepts finite values strictly greater than zero.
///
/// # Errors
/// Returns [`InvalidParameter`] for zero, negative, infinite or NaN values.
pub fn positive(field: &'static str, value: f64) -> Result<(), InvalidParameter> {
    check(
        field,
        value,
        value.is_finite() && value > 0.0,
        "finite and positive",
    )
}

/// Accepts finite values greater than or equal to zero.
///
/// # Errors
/// Returns [`InvalidParameter`] for negative, infinite or NaN values.
pub fn non_negative(field: &'static str, value: f64) -> Result<(), InvalidParameter> {
    check(
        field,
        value,
        value.is_finite() && value >= 0.0,
        "finite and non-negative",
    )
}

/// Accepts any finite value.
///
/// # Errors
/// Returns [`InvalidParameter`] for infinite or NaN values.
pub fn finite(field: &'static str, value: f64) -> Result<(), InvalidParameter> {
    check(field, value, value.is_finite(), "finite")
}

/// Accepts values inside `range`; `requirement` describes the range to the user.
///
/// # Errors
/// Returns [`InvalidParameter`] for values outside `range`, including NaN.
pub fn within(
    field: &'static str,
    value: f64,
    range: &RangeInclusive<f64>,
    requirement: &'static str,
) -> Result<(), InvalidParameter> {
    check(field, value, range.contains(&value), requirement)
}

#[cfg(test)]
mod tests {
    use super::{finite, non_negative, positive, within};

    #[test]
    fn positive_rejects_zero_negative_and_non_finite() {
        assert!(positive("x", 1.0e-12).is_ok());
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(positive("x", bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn non_negative_accepts_zero() {
        assert!(non_negative("x", 0.0).is_ok());
        assert!(non_negative("x", -f64::MIN_POSITIVE).is_err());
    }

    #[test]
    fn finite_and_within_reject_nan() {
        assert!(finite("x", -3.0).is_ok());
        assert!(finite("x", f64::NAN).is_err());
        assert!(within("x", 0.5, &(0.0..=1.0), "in [0, 1]").is_ok());
        assert!(within("x", f64::NAN, &(0.0..=1.0), "in [0, 1]").is_err());
    }

    #[test]
    fn error_message_names_field_and_requirement() {
        let message = positive("caliber", -0.1).unwrap_err().to_string();
        assert!(
            message.contains("caliber") && message.contains("positive"),
            "{message}"
        );
    }
}
