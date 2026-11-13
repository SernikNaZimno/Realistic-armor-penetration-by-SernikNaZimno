//! Profiles of projectile noses as bodies of revolution.

use rap_domain::NoseShape;

/// Axial length of a nose of the given shape on a body of radius
/// `body_radius`, in the same unit as the radius.
#[must_use]
pub fn length(shape: NoseShape, body_radius: f64) -> f64 {
    match shape {
        NoseShape::Flat => 0.0,
        NoseShape::Hemisphere => body_radius,
        NoseShape::Cone { half_angle } => body_radius / half_angle.tan(),
        NoseShape::Ogive {
            caliber_radius_head,
        } => {
            let arc_radius = 2.0 * caliber_radius_head * body_radius;
            (2.0 * arc_radius * body_radius - body_radius.powi(2)).sqrt()
        }
    }
}

/// Radius of the nose at `distance` behind its tip. Behind the nose the
/// body radius is returned; ahead of the tip, zero.
#[must_use]
pub fn radius(shape: NoseShape, body_radius: f64, distance: f64) -> f64 {
    let nose_length = length(shape, body_radius);
    if distance >= nose_length {
        return body_radius;
    }
    if distance <= 0.0 {
        return 0.0;
    }
    match shape {
        NoseShape::Flat => body_radius,
        NoseShape::Hemisphere => (body_radius.powi(2) - (body_radius - distance).powi(2)).sqrt(),
        NoseShape::Cone { half_angle } => distance * half_angle.tan(),
        NoseShape::Ogive {
            caliber_radius_head,
        } => {
            // Circular arc of radius ρ, tangent to the body at the shoulder
            // and centred ρ − R off the axis.
            let arc_radius = 2.0 * caliber_radius_head * body_radius;
            (arc_radius.powi(2) - (nose_length - distance).powi(2)).sqrt()
                - (arc_radius - body_radius)
        }
    }
}

#[cfg(test)]
mod tests {
    use rap_domain::NoseShape;

    use super::{length, radius};

    const BODY_RADIUS: f64 = 0.038;
    const SHAPES: [NoseShape; 4] = [
        NoseShape::Flat,
        NoseShape::Hemisphere,
        NoseShape::Cone { half_angle: 0.3 },
        NoseShape::Ogive {
            caliber_radius_head: 1.5,
        },
    ];

    #[test]
    fn every_nose_starts_at_a_point_or_face_and_ends_at_the_body_radius() {
        for shape in SHAPES {
            let nose_length = length(shape, BODY_RADIUS);
            assert!((radius(shape, BODY_RADIUS, nose_length) - BODY_RADIUS).abs() < 1e-12);
            assert!((radius(shape, BODY_RADIUS, 10.0 * BODY_RADIUS) - BODY_RADIUS).abs() < 1e-12);
            assert!(radius(shape, BODY_RADIUS, -1.0e-3).abs() < 1e-12);
            // The profile never shrinks towards the base.
            let samples: Vec<f64> = (0..=100)
                .map(|step| radius(shape, BODY_RADIUS, nose_length * f64::from(step) / 100.0))
                .collect();
            assert!(samples.is_sorted(), "{shape:?}");
        }
    }

    #[test]
    fn nose_lengths_match_their_definitions() {
        assert!(length(NoseShape::Flat, BODY_RADIUS).abs() < 1e-15);
        assert!((length(NoseShape::Hemisphere, BODY_RADIUS) - BODY_RADIUS).abs() < 1e-15);
        let cone = NoseShape::Cone {
            half_angle: std::f64::consts::FRAC_PI_4,
        };
        assert!((length(cone, BODY_RADIUS) - BODY_RADIUS).abs() < 1e-12);
        // A 2-calibre ogive is √7 body radii long: ρ = 4R, L² = 8R² − R².
        let ogive = NoseShape::Ogive {
            caliber_radius_head: 2.0,
        };
        assert!((length(ogive, BODY_RADIUS) / BODY_RADIUS - 7.0_f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn half_calibre_ogive_is_a_hemisphere() {
        let ogive = NoseShape::Ogive {
            caliber_radius_head: 0.5,
        };
        for step in 0..=20 {
            let distance = BODY_RADIUS * f64::from(step) / 20.0;
            let difference = radius(ogive, BODY_RADIUS, distance)
                - radius(NoseShape::Hemisphere, BODY_RADIUS, distance);
            assert!(difference.abs() < 1e-12);
        }
    }

    #[test]
    fn cone_radius_is_linear() {
        let cone = NoseShape::Cone { half_angle: 0.3 };
        let nose_length = length(cone, BODY_RADIUS);
        assert!((radius(cone, BODY_RADIUS, nose_length / 2.0) - BODY_RADIUS / 2.0).abs() < 1e-12);
    }
}
