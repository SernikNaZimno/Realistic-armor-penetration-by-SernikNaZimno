//! Stress, energy and volume update of one material point.

use glam::DMat2;
use rap_materials::{EquationOfState, Material, PlasticState, StrengthModel};
use rap_solver::{Deviator, Particle, SolverConfig};

/// Longitudinal elastic wave speed, `sqrt(c_bulk² + 4G/(3ρ))`.
pub(crate) fn longitudinal_speed(material: &Material, density: f64, specific_energy: f64) -> f64 {
    let bulk = material.eos.sound_speed(density, specific_energy);
    (bulk.powi(2) + 4.0 * material.strength.shear_modulus() / (3.0 * density)).sqrt()
}

/// Advances the material state of `particle` over `time_step` under the
/// velocity gradient `∂v_i/∂x_j`.
///
/// The deviatoric stress follows a hypoelastic law in the Jaumann rate with
/// von Mises radial return; the pressure comes from the equation of state;
/// the internal energy integrates the stress power, including the artificial
/// viscosity, so shock heating and plastic heating are both accounted for.
///
/// `work_gradient` is the gradient of the time-centred grid velocity. The
/// stress the grid forces were built from, contracted with it, is exactly
/// the work those forces did on the grid, which keeps kinetic plus internal
/// energy conserved by the grid update.
#[expect(
    clippy::similar_names,
    reason = "tensor components are named after their mathematical indices"
)]
pub(crate) fn update(
    particle: &mut Particle,
    material: &Material,
    gradient: DMat2,
    work_gradient: DMat2,
    time_step: f64,
    cell_size: f64,
    config: &SolverConfig,
) {
    let (l_xx, l_yx) = (gradient.x_axis.x, gradient.x_axis.y);
    let (l_xy, l_yy) = (gradient.y_axis.x, gradient.y_axis.y);
    let rate_xy = f64::midpoint(l_xy, l_yx);
    let spin = 0.5 * (l_xy - l_yx);
    let divergence = l_xx + l_yy;
    let mean_rate = divergence / 3.0;

    // Trial elastic deviator: ṡ = 2G·dev(D) + W·s − s·W.
    let shear_modulus = material.strength.shear_modulus();
    let old = particle.deviator;
    let trial = Deviator {
        xx: old.xx + time_step * (2.0 * shear_modulus * (l_xx - mean_rate) + 2.0 * spin * old.xy),
        yy: old.yy + time_step * (2.0 * shear_modulus * (l_yy - mean_rate) - 2.0 * spin * old.xy),
        xy: old.xy + time_step * (2.0 * shear_modulus * rate_xy + spin * (old.yy - old.xx)),
    };

    // Radial return to the yield surface.
    let yield_stress = material.strength.flow_stress(PlasticState {
        strain: particle.plastic_strain,
        strain_rate: particle.plastic_strain_rate,
        temperature: particle.temperature,
    });
    let equivalent = trial.equivalent_stress();
    let (deviator, strain_increment) = if equivalent > yield_stress {
        (
            trial.scaled(yield_stress / equivalent),
            (equivalent - yield_stress) / (3.0 * shear_modulus),
        )
    } else {
        (trial, 0.0)
    };

    // Work of the stress that produced the grid forces, per unit volume.
    let normal = particle.pressure + particle.viscous_pressure;
    let stress_power = (old.xx - normal) * work_gradient.x_axis.x
        + (old.yy - normal) * work_gradient.y_axis.y
        + old.xy * (work_gradient.y_axis.x + work_gradient.x_axis.y);
    particle.specific_energy += time_step * stress_power / particle.density();

    particle.volume *= (time_step * divergence).exp();
    let density = particle.density();

    particle.temperature += material.adiabatic_heating(yield_stress * strain_increment / density);
    particle.plastic_strain += strain_increment;
    particle.plastic_strain_rate = strain_increment / time_step;
    particle.deviator = deviator;
    particle.pressure = material.eos.pressure(density, particle.specific_energy);
    particle.sound_speed = longitudinal_speed(material, density, particle.specific_energy);

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
    use rap_materials::{EquationOfState, Material, PlasticState, StrengthModel, library};
    use rap_solver::{Deviator, Particle, SolverConfig};

    use super::{longitudinal_speed, update};

    const CELL: f64 = 1.0e-3;
    const NO_VISCOSITY: SolverConfig = SolverConfig {
        cfl: 0.3,
        linear_viscosity: 0.0,
        quadratic_viscosity: 0.0,
    };

    fn particle(material: &Material) -> Particle {
        let density = material.eos.reference_density();
        Particle {
            position: DVec2::ZERO,
            velocity: DVec2::ZERO,
            velocity_gradient: DMat2::ZERO,
            mass: density * 1.0e-6,
            volume: 1.0e-6,
            material: 0,
            deviator: Deviator::default(),
            pressure: 0.0,
            viscous_pressure: 0.0,
            specific_energy: 0.0,
            plastic_strain: 0.0,
            plastic_strain_rate: 0.0,
            temperature: 294.0,
            sound_speed: longitudinal_speed(material, density, 0.0),
        }
    }

    /// Velocity gradient with `∂v_x/∂x = xx`, `∂v_x/∂y = xy`, `∂v_y/∂x = yx`.
    fn gradient(xx: f64, xy: f64, yx: f64, yy: f64) -> DMat2 {
        DMat2::from_cols(DVec2::new(xx, yx), DVec2::new(xy, yy))
    }

    #[test]
    fn small_uniaxial_compression_is_linear_elastic() {
        let steel = library::steel_4340();
        let mut state = particle(&steel);
        let strain = -1.0e-4;
        let steps = 100;
        for _ in 0..steps {
            update(
                &mut state,
                &steel,
                gradient(strain / 1.0e-6 / f64::from(steps), 0.0, 0.0, 0.0),
                gradient(strain / 1.0e-6 / f64::from(steps), 0.0, 0.0, 0.0),
                1.0e-6,
                CELL,
                &NO_VISCOSITY,
            );
        }
        let shear = steel.strength.shear_modulus();
        let bulk = steel.eos.reference_density() * 4569.0_f64.powi(2);
        // σ_xx = (K + 4G/3)·ε for uniaxial strain.
        let expected = (bulk + 4.0 * shear / 3.0) * strain;
        assert!(
            (state.stress_xx() / expected - 1.0).abs() < 2e-3,
            "{} vs {expected}",
            state.stress_xx()
        );
        assert!((state.deviator.xx / (4.0 * shear * strain / 3.0) - 1.0).abs() < 1e-6);
        assert!(state.plastic_strain.abs() < f64::EPSILON);
        assert!((state.volume / 1.0e-6 - strain.exp()).abs() < 1e-12);
    }

    #[test]
    fn large_shear_returns_to_the_yield_surface_and_heats() {
        let steel = library::steel_4340();
        let mut state = particle(&steel);
        // Simple shear, 5 % engineering shear strain in one step.
        update(
            &mut state,
            &steel,
            gradient(0.0, 0.05 / 1.0e-7, 0.0, 0.0),
            gradient(0.0, 0.05 / 1.0e-7, 0.0, 0.0),
            1.0e-7,
            CELL,
            &NO_VISCOSITY,
        );
        let initial_yield = steel.strength.flow_stress(PlasticState {
            strain: 0.0,
            strain_rate: 0.0,
            temperature: 294.0,
        });
        assert!((state.deviator.equivalent_stress() / initial_yield - 1.0).abs() < 1e-12);
        assert!(state.plastic_strain > 0.02);
        assert!(state.temperature > 294.0);
        // Shear is isochoric.
        assert!((state.volume - 1.0e-6).abs() < 1e-18);

        // The energy integrates the stress that existed before the step, so
        // it starts rising one step after the stress does. Shearing on at the
        // yield stress τ = Y/√3 then dissipates τ·γ̇ per unit volume.
        assert!(state.specific_energy.abs() < f64::EPSILON);
        let shear_stress = state.deviator.xy;
        let density = state.density();
        let shear_rate = 0.05 / 1.0e-7;
        let continued = gradient(0.0, shear_rate, 0.0, 0.0);
        update(
            &mut state,
            &steel,
            continued,
            continued,
            1.0e-7,
            CELL,
            &NO_VISCOSITY,
        );
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
        let angle_step = std::f64::consts::FRAC_PI_2 / f64::from(steps);
        for _ in 0..steps {
            update(
                &mut state,
                &steel,
                gradient(0.0, -angle_step / 1.0e-9, angle_step / 1.0e-9, 0.0),
                gradient(0.0, -angle_step / 1.0e-9, angle_step / 1.0e-9, 0.0),
                1.0e-9,
                CELL,
                &NO_VISCOSITY,
            );
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
    fn viscosity_acts_only_in_compression() {
        let steel = library::steel_4340();
        let config = SolverConfig::default();
        let mut compressed = particle(&steel);
        update(
            &mut compressed,
            &steel,
            gradient(-1.0e3, 0.0, 0.0, 0.0),
            gradient(-1.0e3, 0.0, 0.0, 0.0),
            1.0e-8,
            CELL,
            &config,
        );
        assert!(compressed.viscous_pressure > 0.0);
        let mut expanded = particle(&steel);
        update(
            &mut expanded,
            &steel,
            gradient(1.0e3, 0.0, 0.0, 0.0),
            gradient(1.0e3, 0.0, 0.0, 0.0),
            1.0e-8,
            CELL,
            &config,
        );
        assert!(expanded.viscous_pressure.abs() < f64::EPSILON);
    }
}
