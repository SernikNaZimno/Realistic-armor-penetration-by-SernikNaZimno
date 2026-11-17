//! What the colours of a picture show, and the colours themselves.

use rap_solver::Particle;

/// A colour as red, green and blue.
pub type Rgb = [u8; 3];

/// Colour stops of the scalar colour map, from low to high (viridis).
const GRADIENT: [[f64; 3]; 5] = [
    [68.0, 1.0, 84.0],
    [59.0, 82.0, 139.0],
    [33.0, 145.0, 140.0],
    [94.0, 201.0, 98.0],
    [253.0, 231.0, 37.0],
];

/// Colours of materials, by material index.
const MATERIAL_COLORS: [[f64; 3]; 6] = [
    [230.0, 145.0, 56.0],
    [120.0, 144.0, 156.0],
    [106.0, 168.0, 79.0],
    [166.0, 77.0, 121.0],
    [61.0, 133.0, 198.0],
    [241.0, 194.0, 50.0],
];

/// Greys of the armor in a three-dimensional picture, by material index.
const ARMOR_GREYS: [f64; 4] = [150.0, 110.0, 185.0, 85.0];

/// Brightness of failed material relative to intact material.
const FAILED_SHADE: f64 = 0.45;
/// Brightness of failed armor relative to intact armor.
const FAILED_ARMOR_SHADE: f64 = 0.6;

/// Quantity shown by the colours of a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The material, and whether it has failed.
    Material,
    /// Accumulated damage, from 0 to 1.
    Damage,
    /// Equivalent plastic strain.
    PlasticStrain,
    /// Pressure, in gigapascals.
    Pressure,
    /// Temperature, in kelvins.
    Temperature,
    /// Speed, in metres per second.
    Speed,
}

impl Field {
    /// Every field, in the order they are offered.
    pub const ALL: [Self; 6] = [
        Self::Material,
        Self::Damage,
        Self::PlasticStrain,
        Self::Pressure,
        Self::Temperature,
        Self::Speed,
    ];

    /// Name of the field with its unit.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Material => "Material",
            Self::Damage => "Damage",
            Self::PlasticStrain => "Plastic strain",
            Self::Pressure => "Pressure [GPa]",
            Self::Temperature => "Temperature [K]",
            Self::Speed => "Speed [m/s]",
        }
    }

    /// Value of the field at a particle, in the unit of the label.
    #[must_use]
    pub fn value(self, particle: &Particle) -> f64 {
        match self {
            Self::Material => 0.0,
            Self::Damage => particle.damage,
            Self::PlasticStrain => particle.plastic_strain,
            Self::Pressure => particle.pressure * 1.0e-9,
            Self::Temperature => particle.temperature,
            Self::Speed => particle.velocity.length(),
        }
    }

    /// Range of values mapped onto the colour scale, never empty.
    #[must_use]
    pub fn range(self, particles: &[Particle]) -> (f64, f64) {
        self.range_of(particles.iter().map(|particle| self.value(particle)))
    }

    /// Range of `values` of this field mapped onto the colour scale, never
    /// empty; damage always runs from 0 to 1.
    #[must_use]
    pub fn range_of(self, values: impl Iterator<Item = f64>) -> (f64, f64) {
        if self == Self::Damage {
            return (0.0, 1.0);
        }
        let (lowest, highest) = values.fold((f64::MAX, f64::MIN), |(lowest, highest), value| {
            (lowest.min(value), highest.max(value))
        });
        if lowest > highest {
            (0.0, 1.0)
        } else if highest - lowest < 1.0e-9 {
            (lowest, lowest + 1.0)
        } else {
            (lowest, highest)
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to the range of a colour channel"
)]
fn channel(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// `rgb` at `brightness` times its own.
#[must_use]
pub fn shaded(rgb: Rgb, brightness: f64) -> Rgb {
    rgb.map(|value| channel(f64::from(value) * brightness))
}

/// Colour of the scalar colour map at `fraction` of the way from its low
/// to its high end.
#[must_use]
pub fn gradient(fraction: f64) -> Rgb {
    let scaled = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0) * 4.0
    };
    let (index, lower) = if scaled >= 3.0 {
        (3, 3.0)
    } else if scaled >= 2.0 {
        (2, 2.0)
    } else if scaled >= 1.0 {
        (1, 1.0)
    } else {
        (0, 0.0)
    };
    let (from, to) = (GRADIENT[index], GRADIENT[index + 1]);
    let blend = scaled - lower;
    [0, 1, 2].map(|part| channel(from[part] + blend * (to[part] - from[part])))
}

/// Colour of a material; failed material is darker.
#[must_use]
pub fn material(material: usize, failed: bool) -> Rgb {
    let shade = if failed { FAILED_SHADE } else { 1.0 };
    MATERIAL_COLORS[material % MATERIAL_COLORS.len()].map(|value| channel(value * shade))
}

/// Grey of an armor material, so that a projectile of the same material
/// stands out against it; failed armor is darker.
#[must_use]
pub fn armor(material: usize, failed: bool) -> Rgb {
    let shade = if failed { FAILED_ARMOR_SHADE } else { 1.0 };
    [channel(ARMOR_GREYS[material % ARMOR_GREYS.len()] * shade); 3]
}

#[cfg(test)]
mod tests {
    use glam::{DMat2, DVec2};
    use rap_solver::{Deviator, Particle};

    use super::{Field, armor, gradient, material, shaded};

    fn particle(damage: f64) -> Particle {
        Particle {
            position: DVec2::ZERO,
            velocity: DVec2::new(300.0, 400.0),
            velocity_gradient: DMat2::ZERO,
            mass: 1.0,
            volume: 1.0e-4,
            body: 0,
            material: 0,
            deviator: Deviator::default(),
            pressure: 2.0e9,
            viscous_pressure: 0.0,
            specific_energy: 0.0,
            plastic_strain: 0.1,
            plastic_strain_rate: 0.0,
            temperature: 500.0,
            damage,
            sound_speed: 5000.0,
        }
    }

    #[test]
    fn gradient_runs_from_its_first_to_its_last_stop() {
        assert_eq!(gradient(0.0), [68, 1, 84]);
        assert_eq!(gradient(1.0), [253, 231, 37]);
        assert_eq!(gradient(0.5), [33, 145, 140]);
        // Out-of-range and undefined values are clamped to the ends.
        assert_eq!(gradient(-3.0), gradient(0.0));
        assert_eq!(gradient(7.0), gradient(1.0));
        assert_eq!(gradient(f64::NAN), gradient(0.0));
    }

    #[test]
    fn failed_material_is_darker_and_colours_repeat() {
        let (intact, failed) = (material(1, false), material(1, true));
        assert!(failed[0] < intact[0] && failed[2] < intact[2]);
        assert_eq!(material(0, false), material(6, false));
        let (plate, broken) = (armor(0, false), armor(0, true));
        assert_eq!(plate, [150; 3]);
        assert!(broken[0] < plate[0]);
        assert_eq!(shaded([200, 100, 50], 0.5), [100, 50, 25]);
        assert_eq!(shaded([200, 100, 50], 9.0), [255; 3]);
    }

    #[test]
    fn field_values_use_display_units() {
        let sample = particle(0.5);
        assert!((Field::Pressure.value(&sample) - 2.0).abs() < 1e-12);
        assert!((Field::Speed.value(&sample) - 500.0).abs() < 1e-9);
        assert!((Field::Damage.value(&sample) - 0.5).abs() < 1e-12);
        assert!((Field::PlasticStrain.value(&sample) - 0.1).abs() < 1e-12);
        assert!(Field::ALL.iter().all(|field| !field.label().is_empty()));
    }

    #[test]
    fn ranges_are_never_empty() {
        assert_eq!(Field::Temperature.range(&[]), (0.0, 1.0));
        let uniform = [particle(0.2), particle(0.4)];
        assert_eq!(Field::Temperature.range(&uniform), (500.0, 501.0));
        assert_eq!(Field::Damage.range(&uniform), (0.0, 1.0));
        assert_eq!(
            Field::Speed.range_of([3.0, -1.0, 2.0].into_iter()),
            (-1.0, 3.0)
        );
    }
}
