//! Stress, energy, volume and damage update of one material point.

use glam::DMat2;
use rap_materials::{DamageModel, EquationOfState, Material, PlasticState, StrengthModel};
use rap_solver::{Deviator, Particle, SolverConfig};

/// Stress triaxiality is limited to this range before it enters the damage
/// model; beyond it the fracture strain is either huge or meaningless.
const TRIAXIALITY_RANGE: (f64, f64) = (-10.0, 1.5);

/// Motion of the continuum at a material point over one step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Kinematics {
    /// In-plane velocity gradient `∂v_i/∂x_j` at the end of the step.
    pub(crate) gradient: DMat2,
    /// In-plane gradient of the time-centred grid velocity.
    pub(crate) work_gradient: DMat2,
    /// Hoop strain rate `v_r/r` at the end of the step; zero in plane strain.
    pub(crate) hoop_rate: f64,
    /// Hoop strain rate of the time-centred grid velocity.
    pub(crate) work_hoop_rate: f64,
}

/// Fastest wave speed in the material: the longitudinal elastic speed
/// `sqrt(c_bulk² + 4G/(3ρ))` while it is intact, the bulk speed once it has
/// failed and lost its shear stiffness.
pub(crate) fn wave_speed(
    material: &Material,
    density: f64,
    specific_energy: f64,
    failed: bool,
) -> f64 {
    let bulk = material.eos.sound_speed(density, specific_energy);
    if failed {
        return bulk;
    }
    (bulk.powi(2) + 4.0 * material.strength.shear_modulus() / (3.0 * density)).sqrt()
}

/// Advances the material state of `particle` over `time_step`.
///
/// The deviatoric stress follows a hypoelastic law in the Jaumann rate with
/// von Mises radial return; the pressure comes from the equation of state;
/// the internal energy integrates the stress power, including the artificial
/// viscosity, so shock heating and plastic heating are both accounted for.
/// Plastic flow accumulates damage, and tension beyond the spall strength
/// fails the point at once; a failed point keeps no deviatoric stress and
/// carries no tension.
///
/// The stress the grid forces were built from, contracted with the
/// time-centred velocity gradient, is exactly the work those forces did on
/// the grid, which keeps kinetic plus internal energy conserved by the grid
/// update.
#[expect(
    clippy::similar_names,
    reason = "tensor components are named after their mathematical indices"
)]
pub(crate) fn update(
    particle: &mut Particle,
    material: &Material,
    kinematics: &Kinematics,
    time_step: f64,
    cell_size: f64,
    config: &SolverConfig,
) {
    let gradient = kinematics.gradient;
    let (l_xx, l_yx) = (gradient.x_axis.x, gradient.x_axis.y);
    let (l_xy, l_yy) = (gradient.y_axis.x, gradient.y_axis.y);
    let rate_xy = f64::midpoint(l_xy, l_yx);
    let spin = 0.5 * (l_xy - l_yx);
    let divergence = l_xx + l_yy + kinematics.hoop_rate;
    let mean_rate = divergence / 3.0;

    // Work of the stress that produced the grid forces, per unit volume.
    let old = particle.deviator;
    let normal = particle.pressure + particle.viscous_pressure;
    let work = kinematics.work_gradient;
    let stress_power = (old.xx - normal) * work.x_axis.x
        + (old.yy - normal) * work.y_axis.y
        + old.xy * (work.y_axis.x + work.x_axis.y)
        + (old.zz() - normal) * kinematics.work_hoop_rate;
    particle.specific_energy += time_step * stress_power / particle.density();

    particle.volume *= (time_step * divergence).exp();
    let density = particle.density();

    if particle.is_failed() {
        particle.plastic_strain_rate = 0.0;
    } else {
        // Trial elastic deviator: ṡ = 2G·dev(D) + W·s − s·W. The hoop
        // component follows from the zero trace.
        let shear_modulus = material.strength.shear_modulus();
        let trial = Deviator {
            xx: old.xx
                + time_step * (2.0 * shear_modulus * (l_xx - mean_rate) + 2.0 * spin * old.xy),
            yy: old.yy
                + time_step * (2.0 * shear_modulus * (l_yy - mean_rate) - 2.0 * spin * old.xy),
            xy: old.xy + time_step * (2.0 * shear_modulus * rate_xy + spin * (old.yy - old.xx)),
        };

        // Radial return to the yield surface.
        let state = PlasticState {
            strain: particle.plastic_strain,
            strain_rate: particle.plastic_strain_rate,
            temperature: particle.temperature,
        };
        let yield_stress = material.strength.flow_stress(state);
        let equivalent = trial.equivalent_stress();
        if equivalent > yield_stress {
            let strain_increment = (equivalent - yield_stress) / (3.0 * shear_modulus);
            let triaxiality = (-particle.pressure / yield_stress.max(1.0))
                .clamp(TRIAXIALITY_RANGE.0, TRIAXIALITY_RANGE.1);
            let damage = material
                .damage
                .damage_increment(strain_increment, triaxiality, state);
            particle.damage = (particle.damage + damage).min(1.0);
            particle.deviator = trial.scaled(yield_stress / equivalent);
            particle.temperature +=
                material.adiabatic_heating(yield_stress * strain_increment / density);
            particle.plastic_strain += strain_increment;
            particle.plastic_strain_rate = strain_increment / time_step;
        } else {
            particle.deviator = trial;
            particle.plastic_strain_rate = 0.0;
        }
    }

    particle.pressure = material.eos.pressure(density, particle.specific_energy);
    if particle.pressure < -material.spall_strength {
        particle.damage = 1.0;
    }
    if particle.is_failed() {
        particle.deviator = Deviator::default();
        particle.pressure = particle.pressure.max(0.0);
    }
    particle.sound_speed = wave_speed(
        material,
        density,
        particle.specific_energy,
        particle.is_failed(),
    );

    let compression_rate = (-divergence).max(0.0);
    particle.viscous_pressure = density
        * cell_size
        * compression_rate
        * (config.linear_viscosity * particle.sound_speed
            + config.quadratic_viscosity * cell_size * compression_rate);
}

#[cfg(test)]
mod tests {
    use glam::{DMat2, DVec2};
    use rap_materials::{Eos, EquationOfState, Material, PlasticState, StrengthModel, library};
    use rap_solver::{Deviator, Particle, SolverConfig};

    use super::{Kinematics, update, wave_speed};

    const CELL: f64 = 1.0e-3;
    const NO_VISCOSITY: SolverConfig = SolverConfig {
        cfl: 0.3,
        linear_viscosity: 0.0,
        quadratic_viscosity: 0.0,
        friction: 0.0,
    };

    fn particle(material: &Material) -> Particle {
        let density = material.eos.reference_density();
        Particle {
            position: DVec2::ZERO,
            velocity: DVec2::ZERO,
            velocity_gradient: DMat2::ZERO,
            mass: density * 1.0e-6,
            volume: 1.0e-6,
            body: 0,
            material: 0,
            deviator: Deviator::default(),
            pressure: 0.0,
            viscous_pressure: 0.0,
            specific_energy: 0.0,
            plastic_strain: 0.0,
            plastic_strain_rate: 0.0,
            temperature: 294.0,
            damage: 0.0,
            sound_speed: wave_speed(material, density, 0.0, false),
        }
    }

    /// Plane motion with `∂v_x/∂x = xx`, `∂v_x/∂y = xy`, `∂v_y/∂x = yx`,
    /// `∂v_y/∂y = yy`, the same at the end of the step and time-centred.
    fn plane(xx: f64, xy: f64, yx: f64, yy: f64) -> Kinematics {
        let gradient = DMat2::from_cols(DVec2::new(xx, yx), DVec2::new(xy, yy));
        Kinematics {
            gradient,
            work_gradient: gradient,
            hoop_rate: 0.0,
            work_hoop_rate: 0.0,
        }
    }

    fn advance(state: &mut Particle, material: &Material, kinematics: &Kinematics, step: f64) {
        update(state, material, kinematics, step, CELL, &NO_VISCOSITY);
    }

    #[test]
    fn small_uniaxial_compression_is_linear_elastic() {
        let steel = library::steel_4340();
        let mut state = particle(&steel);
        let strain = -1.0e-4;
        let steps = 100;
        let kinematics = plane(strain / 1.0e-6 / f64::from(steps), 0.0, 0.0, 0.0);
        for _ in 0..steps {
            advance(&mut state, &steel, &kinematics, 1.0e-6);
        }
        let shear = steel.strength.shear_modulus();
        let bulk = steel.eos.reference_density() * 4569.0_f64.powi(2);
        // σ_xx = (K + 4G/3)·ε for uniaxial strain.
        let expected = (bulk + 4.0 * shear / 3.0) * strain;
        assert!(
            (state.stress_xx() / expected - 1.0).abs() < 2e-3,
            "{}",
            state.stress_xx()
        );
        assert!((state.deviator.xx / (4.0 * shear * strain / 3.0) - 1.0).abs() < 1e-6);
        assert!(state.plastic_strain.abs() < f64::EPSILON);
        assert!(state.damage.abs() < f64::EPSILON);
        assert!((state.volume / 1.0e-6 - strain.exp()).abs() < 1e-12);
    }

    #[test]
    fn hoop_strain_changes_volume_and_loads_the_hoop_direction() {
        let steel = library::steel_4340();
        let mut state = particle(&steel);
        // Uniform radial expansion seen at one point: only v_r/r is non-zero.
        let kinematics = Kinematics {
            gradient: DMat2::ZERO,
            work_gradient: DMat2::ZERO,
            hoop_rate: 100.0,
            work_hoop_rate: 100.0,
        };
        advance(&mut state, &steel, &kinematics, 1.0e-7);
        let strain: f64 = 100.0 * 1.0e-7;
        assert!((state.volume / 1.0e-6 - strain.exp()).abs() < 1e-12);
        let shear = steel.strength.shear_modulus();
        // dev(D)_θθ = 2ε/3, so s_θθ = 4Gε/3 and s_xx = s_yy = −2Gε/3.
        assert!((state.deviator.zz() / (4.0 * shear * strain / 3.0) - 1.0).abs() < 1e-9);
        assert!((state.deviator.xx / (-2.0 * shear * strain / 3.0) - 1.0).abs() < 1e-9);
        assert!(state.pressure < 0.0);
    }

    #[test]
    fn large_shear_returns_to_the_yield_surface_and_heats() {
        let steel = library::steel_4340();
        let mut state = particle(&steel);
        // Simple shear, 5 % engineering shear strain in one step.
        let shear_rate = 0.05 / 1.0e-7;
        let kinematics = plane(0.0, shear_rate, 0.0, 0.0);
        advance(&mut state, &steel, &kinematics, 1.0e-7);
        let initial_yield = steel.strength.flow_stress(PlasticState {
            strain: 0.0,
            strain_rate: 0.0,
            temperature: 294.0,
        });
        assert!((state.deviator.equivalent_stress() / initial_yield - 1.0).abs() < 1e-12);
        assert!(state.plastic_strain > 0.02);
        assert!(state.temperature > 294.0);
        assert!(state.damage > 0.0 && state.damage < 1.0);
        // Shear is isochoric.
        assert!((state.volume - 1.0e-6).abs() < 1e-18);

        // The energy integrates the stress that existed before the step, so
        // it starts rising one step after the stress does. Shearing on at the
        // yield stress τ = Y/√3 then dissipates τ·γ̇ per unit volume.
        assert!(state.specific_energy.abs() < f64::EPSILON);
        let shear_stress = state.deviator.xy;
        let density = state.density();
        advance(&mut state, &steel, &kinematics, 1.0e-7);
        let expected = shear_stress * shear_rate * 1.0e-7 / density;
        assert!((state.specific_energy / expected - 1.0).abs() < 1e-12);
    }

    #[test]
    fn rigid_rotation_rotates_the_stress() {
        let steel = library::steel_4340();
        let mut state = particle(&steel);
        state.deviator = Deviator {
            xx: 2.0e8,
            yy: -1.0e8,
            xy: 0.0,
        };
        // Rotate by 90° counter-clockwise: v = ω × r, ∂v_x/∂y = −ω, ∂v_y/∂x = ω.
        let steps = 20_000;
        let spin = std::f64::consts::FRAC_PI_2 / f64::from(steps) / 1.0e-9;
        let kinematics = plane(0.0, -spin, spin, 0.0);
        for _ in 0..steps {
            advance(&mut state, &steel, &kinematics, 1.0e-9);
        }
        assert!(
            (state.deviator.xx / -1.0e8 - 1.0).abs() < 1e-3,
            "{:?}",
            state.deviator
        );
        assert!(
            (state.deviator.yy / 2.0e8 - 1.0).abs() < 1e-3,
            "{:?}",
            state.deviator
        );
        assert!(state.deviator.xy.abs() < 1e-3 * 2.0e8);
        assert!(state.plastic_strain.abs() < f64::EPSILON);
    }

    #[test]
    fn sustained_shear_fails_the_material() {
        // Without thermal pressure, shearing at constant volume keeps the
        // pressure, and with it the triaxiality, at zero. (With it, the
        // heated material is confined and becomes far more ductile.)
        let mut steel = library::steel_4340();
        let Eos::MieGruneisen(mut eos) = steel.eos else {
            panic!("built-in steel uses the Mie–Grüneisen equation of state");
        };
        eos.gruneisen_gamma = 0.0;
        steel.eos = Eos::MieGruneisen(eos);
        let mut state = particle(&steel);
        let kinematics = plane(0.0, 1.0e6, 0.0, 0.0);
        let mut steps = 0;
        while !state.is_failed() {
            advance(&mut state, &steel, &kinematics, 1.0e-8);
            steps += 1;
            assert!(steps < 100_000, "the material never failed");
        }
        // Zero triaxiality: failure strain is D1 + D2 = 3.49 before the rate
        // and temperature factors raise it.
        assert!(state.plastic_strain > 3.49, "{}", state.plastic_strain);
        assert!(state.deviator.equivalent_stress().abs() < f64::EPSILON);

        // A failed point resists compression but not tension or shear.
        advance(&mut state, &steel, &plane(-1.0e5, 3.0e5, 0.0, 0.0), 1.0e-7);
        assert!(state.pressure > 0.0);
        assert!(state.deviator.equivalent_stress().abs() < f64::EPSILON);
        advance(&mut state, &steel, &plane(5.0e5, 0.0, 0.0, 0.0), 1.0e-7);
        assert!(state.pressure.abs() < f64::EPSILON);
    }

    #[test]
    fn tension_beyond_the_spall_strength_fails_the_material() {
        let steel = library::steel_4340();
        let mut state = particle(&steel);
        // Uniform expansion in the plane; 1 % volume strain is about 1.6 GPa
        // of tension, so the 3.5 GPa spall strength is passed in the third
        // step.
        let kinematics = plane(0.5e5, 0.0, 0.0, 0.5e5);
        advance(&mut state, &steel, &kinematics, 1.0e-7);
        advance(&mut state, &steel, &kinematics, 1.0e-7);
        assert!(!state.is_failed());
        assert!(
            state.pressure < -3.0e9 && state.pressure > -3.5e9,
            "{}",
            state.pressure
        );
        let intact_speed = state.sound_speed;
        advance(&mut state, &steel, &kinematics, 1.0e-7);
        assert!(state.is_failed());
        assert!(state.pressure.abs() < f64::EPSILON);
        assert!(state.deviator.equivalent_stress().abs() < f64::EPSILON);
        // Without shear stiffness only the bulk wave is left.
        assert!(state.sound_speed < 0.9 * intact_speed);
    }

    #[test]
    fn viscosity_acts_only_in_compression() {
        let steel = library::steel_4340();
        let config = SolverConfig::default();
        let mut compressed = particle(&steel);
        update(
            &mut compressed,
            &steel,
            &plane(-1.0e3, 0.0, 0.0, 0.0),
            1.0e-8,
            CELL,
            &config,
        );
        assert!(compressed.viscous_pressure > 0.0);
        let mut expanded = particle(&steel);
        update(
            &mut expanded,
            &steel,
            &plane(1.0e3, 0.0, 0.0, 0.0),
            1.0e-8,
            CELL,
            &config,
        );
        assert!(expanded.viscous_pressure.abs() < f64::EPSILON);
    }
}
