//! Brittle fracture: how fast a material point breaks once its largest
//! principal stress has passed the tensile strength.
//!
//! Breaking a point the instant the stress passes the limit lets a single
//! stress wave shatter everything it runs through, because no point has
//! time to be unloaded by its neighbours giving way. A crack, however,
//! cannot run faster than a fraction of the wave speed. The damage of an
//! overstressed point therefore grows at the rate at which a crack of that
//! speed would cross a grid cell, in proportion to the overstress; a brief
//! overstress leaves the point weakened but whole.

/// Speed of a running crack as a fraction of the longitudinal wave speed.
pub const CRACK_SPEED_FRACTION: f64 = 0.2;

/// Damage added over `time_step` to a point whose largest principal stress
/// is `stress`, for a tensile strength of `limit`, a wave speed of
/// `wave_speed` and a grid cell of `cell_size`. Zero below the limit.
#[must_use]
pub fn damage_increment(
    stress: f64,
    limit: f64,
    wave_speed: f64,
    cell_size: f64,
    time_step: f64,
) -> f64 {
    let overstress = stress / limit - 1.0;
    if overstress > 0.0 {
        CRACK_SPEED_FRACTION * wave_speed * time_step / cell_size * overstress
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::damage_increment;

    #[test]
    fn damage_grows_only_above_the_limit_and_with_the_overstress() {
        let at = |stress: f64| damage_increment(stress, 2.0e9, 5000.0, 1.0e-3, 1.0e-8);
        assert!(at(1.9e9).abs() < f64::EPSILON);
        assert!(at(2.0e9).abs() < f64::EPSILON);
        assert!(at(-5.0e9).abs() < f64::EPSILON);
        // 50 % over: a crack at a fifth of 5000 m/s crosses a hundredth of
        // a 1 mm cell in 10 ns, and half of that counts.
        assert!((at(3.0e9) - 0.005).abs() < 1e-12);
        assert!((at(4.0e9) / at(3.0e9) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn time_to_break_scales_with_the_cell() {
        // The same overstress breaks a cell twice the size in twice the time.
        let small = damage_increment(3.0e9, 2.0e9, 5000.0, 1.0e-3, 1.0e-8);
        let large = damage_increment(3.0e9, 2.0e9, 5000.0, 2.0e-3, 2.0e-8);
        assert!((small - large).abs() < 1e-15);
    }
}
