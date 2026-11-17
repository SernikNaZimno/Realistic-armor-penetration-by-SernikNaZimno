//! Binary layout of the buffers shared with the shaders: what both models
//! use (`common.wgsl`) and the two-dimensional model (`flat.wgsl`).
//!
//! Every buffer is a sequence of 32-bit words; floats are stored by their
//! bit patterns. The word offsets below mirror the WGSL struct layouts.

use glam::{DMat2, DVec2};
use rap_materials::{Damage, Eos, EquationOfState, Material, Strength, StrengthModel};
use rap_solver::{Deviator, Model, Particle, SolverError, Symmetry, Wall};

/// Words per `Particle` in the shader.
pub(crate) const PARTICLE_WORDS: usize = 24;
/// Words per `Material` in the shader.
const MATERIAL_WORDS: usize = 45;
/// What the shader gets for a material without a tensile strength: a
/// stress no material reaches.
const NO_TENSILE_LIMIT: f64 = 1.0e30;
/// Words in the shader's `Clock`.
pub(crate) const CLOCK_WORDS: usize = 8;

/// Particle status values, as in the shader.
pub(crate) const ACTIVE: u32 = 0;
pub(crate) const DEPARTED: u32 = 1;

/// Converts to single precision.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the GPU solver works in single precision by design"
)]
pub(crate) fn single(value: f64) -> f32 {
    value as f32
}

pub(crate) fn bits(value: f64) -> u32 {
    single(value).to_bits()
}

pub(crate) fn real(word: u32) -> f64 {
    f64::from(f32::from_bits(word))
}

pub(crate) fn index(value: usize) -> u32 {
    u32::try_from(value).expect("indices in a validated model fit in 32 bits")
}

pub(crate) fn wall_code(wall: Wall) -> u32 {
    match wall {
        Wall::Frictionless => 0,
        Wall::Sliding => 1,
        Wall::Sticky => 2,
        Wall::Outflow => 3,
    }
}

/// The shader's `Params`.
pub(crate) fn encode_params(model: &Model) -> Vec<u32> {
    let [nodes_x, nodes_y] = model.grid.nodes();
    let (lower, upper) = model.grid.interior();
    let axisymmetric = model.symmetry == Symmetry::Axisymmetric;
    vec![
        index(nodes_x),
        index(nodes_y),
        index(model.body_count()),
        index(model.seeds.len()),
        bits(lower.x),
        bits(lower.y),
        bits(upper.x),
        bits(upper.y),
        bits(model.grid.cell_size),
        bits(model.config.cfl),
        bits(model.config.linear_viscosity),
        bits(model.config.quadratic_viscosity),
        bits(model.config.friction),
        bits(lower.y),
        u32::from(axisymmetric),
        wall_code(model.walls.left),
        wall_code(model.walls.right),
        wall_code(model.walls.bottom),
        wall_code(model.walls.top),
        0,
    ]
}

/// The shader's `array<Material>`.
///
/// # Errors
/// Returns [`SolverError::Gpu`] for a material model the shader does not
/// implement.
pub(crate) fn encode_materials(materials: &[Material]) -> Result<Vec<u32>, SolverError> {
    let mut words = Vec::with_capacity(materials.len() * MATERIAL_WORDS);
    for material in materials {
        words.extend(encode_material(material)?);
    }
    Ok(words)
}

/// Model selectors of the shader's `Material`.
const FIRST_MODEL: u32 = 0;
const SECOND_MODEL: u32 = 1;

/// Parameters of the equation of state: selector, reference density, then
/// the Mie-Grüneisen and the polynomial parameters.
fn encode_eos(eos: &Eos) -> Option<(u32, f64, [f64; 6])> {
    match eos {
        Eos::MieGruneisen(eos) => Some((
            FIRST_MODEL,
            eos.reference_density,
            [
                eos.bulk_sound_speed,
                eos.hugoniot_slope,
                eos.gruneisen_gamma,
                0.0,
                0.0,
                0.0,
            ],
        )),
        Eos::Polynomial(eos) => Some((
            SECOND_MODEL,
            eos.reference_density,
            [0.0, 0.0, 0.0, eos.k1, eos.k2, eos.k3],
        )),
        _ => None,
    }
}

/// Parameters of the strength model: selector, then the Johnson-Cook and
/// the Johnson-Holmquist parameters.
fn encode_strength(strength: &Strength) -> Option<(u32, [f64; 8], [f64; 10])> {
    match strength {
        Strength::JohnsonCook(strength) => Some((
            FIRST_MODEL,
            [
                strength.yield_stress,
                strength.hardening_modulus,
                strength.hardening_exponent,
                strength.rate_coefficient,
                strength.thermal_exponent,
                strength.reference.strain_rate,
                strength.reference.temperature,
                strength.reference.melting_temperature,
            ],
            [0.0; 10],
        )),
        Strength::JohnsonHolmquist(strength) => Some((
            SECOND_MODEL,
            [0.0; 8],
            [
                strength.hel_stress,
                strength.hel_pressure,
                strength.tensile_strength,
                strength.intact_coefficient,
                strength.intact_exponent,
                strength.fractured_coefficient,
                strength.fractured_exponent,
                strength.fractured_limit,
                strength.rate_coefficient,
                strength.reference_strain_rate,
            ],
        )),
        _ => None,
    }
}

/// Parameters of the damage model: selector, then the Johnson-Cook and the
/// Johnson-Holmquist parameters.
fn encode_damage(damage: &Damage) -> Option<(u32, [f64; 8], [f64; 4])> {
    match damage {
        Damage::JohnsonCook(damage) => Some((
            FIRST_MODEL,
            [
                damage.d1,
                damage.d2,
                damage.d3,
                damage.d4,
                damage.d5,
                damage.reference.strain_rate,
                damage.reference.temperature,
                damage.reference.melting_temperature,
            ],
            [0.0; 4],
        )),
        Damage::JohnsonHolmquist(damage) => Some((
            SECOND_MODEL,
            [0.0; 8],
            [
                damage.d1,
                damage.d2,
                damage.hel_pressure,
                damage.tensile_strength,
            ],
        )),
        _ => None,
    }
}

fn encode_material(material: &Material) -> Result<Vec<u32>, SolverError> {
    let unsupported = || {
        SolverError::Gpu(format!(
            "material `{}` uses a model the GPU solver does not implement",
            material.id
        ))
    };
    let (eos_model, density, eos) = encode_eos(&material.eos).ok_or_else(unsupported)?;
    let (strength_model, ductile_strength, brittle_strength) =
        encode_strength(&material.strength).ok_or_else(unsupported)?;
    let (damage_model, ductile_damage, brittle_damage) =
        encode_damage(&material.damage).ok_or_else(unsupported)?;
    let mut words = vec![eos_model, strength_model, damage_model, bits(density)];
    let reals = eos
        .into_iter()
        .chain([material.strength.shear_modulus()])
        .chain(ductile_strength)
        .chain(ductile_damage)
        .chain(brittle_strength)
        .chain(brittle_damage)
        .chain([
            material.spall_strength,
            material.specific_heat,
            material.taylor_quinney,
            material.tensile_strength.unwrap_or(NO_TENSILE_LIMIT),
        ]);
    words.extend(reals.map(bits));
    debug_assert_eq!(words.len(), MATERIAL_WORDS);
    Ok(words)
}

/// Longitudinal elastic wave speed of a material in its reference state.
pub(crate) fn reference_wave_speed(material: &Material) -> f64 {
    let density = material.eos.reference_density();
    let bulk = material.eos.sound_speed(density, 0.0);
    (bulk.powi(2) + 4.0 * material.strength.shear_modulus() / (3.0 * density)).sqrt()
}

/// The shader's `array<Particle>`, every particle in its reference state.
pub(crate) fn encode_particles(model: &Model) -> Vec<u32> {
    let mut words = Vec::with_capacity(model.seeds.len() * PARTICLE_WORDS);
    for seed in &model.seeds {
        let material = &model.materials[seed.material];
        let mass = material.eos.reference_density() * seed.volume;
        words.extend([
            bits(seed.position.x),
            bits(seed.position.y),
            bits(seed.velocity.x),
            bits(seed.velocity.y),
            // Velocity gradient.
            0,
            0,
            0,
            0,
            bits(mass),
            bits(seed.volume),
            // Pressure and viscous pressure.
            0,
            0,
            // Deviator and specific energy.
            0,
            0,
            0,
            0,
            // Plastic strain and its rate.
            0,
            0,
            bits(model.initial_temperature),
            // Damage.
            0,
            bits(reference_wave_speed(material)),
            index(seed.body),
            index(seed.material),
            ACTIVE,
        ]);
    }
    words
}

/// Largest sum of wave speed and speed of any particle of `model` in its
/// reference state.
pub(crate) fn fastest_signal(model: &Model) -> f64 {
    model
        .seeds
        .iter()
        .map(|seed| reference_wave_speed(&model.materials[seed.material]) + seed.velocity.length())
        .fold(0.0, f64::max)
}

/// Decodes one shader `Particle` and its status.
pub(crate) fn decode_particle(words: &[u32]) -> (Particle, u32) {
    let particle = Particle {
        position: DVec2::new(real(words[0]), real(words[1])),
        velocity: DVec2::new(real(words[2]), real(words[3])),
        velocity_gradient: DMat2::from_cols(
            DVec2::new(real(words[4]), real(words[5])),
            DVec2::new(real(words[6]), real(words[7])),
        ),
        mass: real(words[8]),
        volume: real(words[9]),
        pressure: real(words[10]),
        viscous_pressure: real(words[11]),
        deviator: Deviator {
            xx: real(words[12]),
            yy: real(words[13]),
            xy: real(words[14]),
        },
        specific_energy: real(words[15]),
        plastic_strain: real(words[16]),
        plastic_strain_rate: real(words[17]),
        temperature: real(words[18]),
        damage: real(words[19]),
        sound_speed: real(words[20]),
        body: words[21] as usize,
        material: words[22] as usize,
    };
    (particle, words[23])
}

/// The shader's `Clock`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Clock {
    pub(crate) time: f32,
    pub(crate) end_time: f32,
    /// Largest `sound speed + speed` of any particle after the last step.
    pub(crate) fastest: f32,
    pub(crate) lost: u32,
    pub(crate) steps: u32,
}

impl Clock {
    /// The clock before the first step of a model whose fastest particle
    /// has `fastest` as the sum of its wave speed and its speed.
    pub(crate) fn starting_with(fastest: f64) -> Self {
        Self {
            time: 0.0,
            end_time: 0.0,
            fastest: single(fastest),
            lost: 0,
            steps: 0,
        }
    }

    pub(crate) fn encode(&self) -> [u32; CLOCK_WORDS] {
        [
            self.time.to_bits(),
            // Summation compensation and the time step are set by the shader.
            0,
            0,
            self.end_time.to_bits(),
            self.fastest.to_bits(),
            0,
            self.lost,
            self.steps,
        ]
    }

    pub(crate) fn decode(words: &[u32]) -> Self {
        Self {
            time: f32::from_bits(words[0]),
            end_time: f32::from_bits(words[3]),
            fastest: f32::from_bits(words[4]),
            lost: words[6],
            steps: words[7],
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use rap_materials::library;
    use rap_solver::{Grid, Model, ParticleSeed, SolverConfig, Symmetry, Wall, Walls};

    use super::{
        ACTIVE, CLOCK_WORDS, Clock, MATERIAL_WORDS, PARTICLE_WORDS, decode_particle,
        encode_materials, encode_params, encode_particles, fastest_signal,
    };

    fn model() -> Model {
        Model {
            grid: Grid {
                cells: [10, 8],
                cell_size: 1.0e-3,
            },
            symmetry: Symmetry::Axisymmetric,
            walls: Walls {
                bottom: Wall::Sliding,
                right: Wall::Outflow,
                ..Walls::default()
            },
            config: SolverConfig::default(),
            materials: vec![library::tungsten_heavy_alloy(), library::steel_4340()],
            seeds: vec![
                ParticleSeed {
                    position: DVec2::new(5.0e-3, 4.0e-3),
                    velocity: DVec2::new(1500.0, 0.0),
                    volume: 2.5e-10,
                    material: 0,
                    body: 0,
                },
                ParticleSeed {
                    position: DVec2::new(6.0e-3, 3.0e-3),
                    velocity: DVec2::ZERO,
                    volume: 5.0e-10,
                    material: 1,
                    body: 1,
                },
            ],
            initial_temperature: 294.0,
        }
    }

    #[test]
    fn buffers_have_the_sizes_the_shader_expects() {
        let model = model();
        // `Params` is 80 bytes in WGSL.
        assert_eq!(encode_params(&model).len() * 4, 80);
        assert_eq!(
            encode_materials(&model.materials).expect("supported").len(),
            2 * MATERIAL_WORDS
        );
        assert_eq!(encode_particles(&model).len(), 2 * PARTICLE_WORDS);
        assert_eq!(Clock::starting_with(1.0).encode().len(), CLOCK_WORDS);
        // `Particle` is 96 bytes and `Material` 180.
        assert_eq!(PARTICLE_WORDS * 4, 96);
        assert_eq!(MATERIAL_WORDS * 4, 180);
    }

    #[test]
    fn params_carry_grid_symmetry_and_walls() {
        let words = encode_params(&model());
        assert_eq!(&words[..4], &[11, 9, 2, 2]);
        assert!((f32::from_bits(words[8]) - 1.0e-3).abs() < 1e-9);
        // Axis at the lower wall plane, axisymmetric, walls L R B T.
        assert!((f32::from_bits(words[13]) - 2.0e-3).abs() < 1e-9);
        assert_eq!(&words[14..19], &[1, 0, 3, 1, 0]);
    }

    #[test]
    fn particles_survive_encoding_to_single_precision() {
        let model = model();
        let words = encode_particles(&model);
        let (particle, status) = decode_particle(&words[PARTICLE_WORDS..]);
        assert_eq!(status, ACTIVE);
        assert_eq!((particle.body, particle.material), (1, 1));
        assert!((particle.position - DVec2::new(6.0e-3, 3.0e-3)).length() < 1e-9);
        assert!((particle.mass / (7830.0 * 5.0e-10) - 1.0).abs() < 1e-6);
        assert!((particle.temperature - 294.0).abs() < 1e-4);
        assert!(particle.sound_speed > 5000.0);
    }

    #[test]
    fn clock_starts_with_the_fastest_signal_and_round_trips() {
        let clock = Clock::starting_with(fastest_signal(&model()));
        // Tungsten wave speed plus the 1500 m/s of the first particle.
        assert!(clock.fastest > 1500.0 + 4000.0);
        assert_eq!(Clock::decode(&clock.encode()), clock);
    }

    #[test]
    fn materials_carry_their_model_and_only_its_parameters() {
        let materials = [library::steel_4340(), library::alumina()];
        let words = encode_materials(&materials).expect("supported");
        let real = |word: u32| f64::from(f32::from_bits(word));
        let (steel, alumina) = words.split_at(MATERIAL_WORDS);
        // Selectors: equation of state, strength, damage.
        assert_eq!(&steel[..3], &[0, 0, 0]);
        assert_eq!(&alumina[..3], &[1, 1, 1]);
        // Steel: sound speed set, polynomial coefficients and JH-2 unused.
        assert!((real(steel[4]) - 4569.0).abs() < 1e-3);
        assert!(steel[7..10].iter().all(|word| *word == 0));
        assert!(steel[27..41].iter().all(|word| *word == 0));
        // Alumina: the reverse, with the bulk modulus first.
        assert!(alumina[4..7].iter().all(|word| *word == 0));
        assert!(real(alumina[7]) > 1.0e11);
        assert!(alumina[11..27].iter().all(|word| *word == 0));
        assert!(real(alumina[27]) > 1.0e9 && real(alumina[28]) > 1.0e9);
        // Both end with spall strength, specific heat and Taylor-Quinney.
        for material in [steel, alumina] {
            assert!(real(material[41]) > 0.0 && real(material[42]) > 100.0);
            assert!(real(material[43]) > 0.0 && real(material[43]) <= 1.0);
            // Neither has a tensile strength.
            assert!(real(material[44]) > 1.0e29);
        }
    }
}
