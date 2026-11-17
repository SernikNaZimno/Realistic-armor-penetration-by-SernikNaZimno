// What the two- and three-dimensional solvers share: the material models,
// the clock and the time step. This text is placed in front of `flat.wgsl`
// or `solid.wgsl`, which declare the buffers (`params`, `clock`, ...) the
// functions here refer to.

const MAX_BODIES: u32 = 8u;
// Invocations per workgroup of the passes indexed by `linear_index`.
const LINEAR_GROUP: u32 = 64u;
const WALL_CELLS: u32 = 2u;

const WALL_FRICTIONLESS: u32 = 0u;
const WALL_SLIDING: u32 = 1u;
const WALL_STICKY: u32 = 2u;
const WALL_OUTFLOW: u32 = 3u;

const ACTIVE: u32 = 0u;
const DEPARTED: u32 = 1u;
const LOST: u32 = 2u;

const EOS_MIE_GRUNEISEN: u32 = 0u;
const EOS_POLYNOMIAL: u32 = 1u;
const STRENGTH_JOHNSON_COOK: u32 = 0u;
const STRENGTH_JOHNSON_HOLMQUIST: u32 = 1u;
const DAMAGE_JOHNSON_COOK: u32 = 0u;
const DAMAGE_JOHNSON_HOLMQUIST: u32 = 1u;

// How much further apart than touching, in cells, the centres of two
// bodies may be and still count as in contact.
const CONTACT_MARGIN: f32 = 0.02;

// Smallest fracture strain, so that damage increments stay finite.
const MIN_FAILURE_STRAIN: f32 = 1.0e-6;

// Parameters of every model a material may use; the selectors say which
// ones apply, and the others are zero.
struct Material {
    eos_model: u32,
    strength_model: u32,
    damage_model: u32,
    density: f32,
    // Mie-Grüneisen equation of state.
    sound_speed: f32,
    hugoniot_slope: f32,
    gruneisen_gamma: f32,
    // Polynomial equation of state.
    k1: f32,
    k2: f32,
    k3: f32,
    shear_modulus: f32,
    // Johnson-Cook strength.
    yield_stress: f32,
    hardening_modulus: f32,
    hardening_exponent: f32,
    rate_coefficient: f32,
    thermal_exponent: f32,
    strength_rate: f32,
    strength_temperature: f32,
    strength_melting: f32,
    // Johnson-Cook damage.
    d1: f32,
    d2: f32,
    d3: f32,
    d4: f32,
    d5: f32,
    damage_rate: f32,
    damage_temperature: f32,
    damage_melting: f32,
    // Johnson-Holmquist (JH-2) strength.
    hel_stress: f32,
    hel_pressure: f32,
    tensile_strength: f32,
    intact_coefficient: f32,
    intact_exponent: f32,
    fractured_coefficient: f32,
    fractured_exponent: f32,
    fractured_limit: f32,
    brittle_rate_coefficient: f32,
    brittle_rate: f32,
    // Johnson-Holmquist (JH-2) damage.
    brittle_d1: f32,
    brittle_d2: f32,
    damage_hel_pressure: f32,
    damage_tensile_strength: f32,
    spall_strength: f32,
    specific_heat: f32,
    taylor_quinney: f32,
}

struct Clock {
    time: f32,
    compensation: f32,
    time_step: f32,
    end_time: f32,
    // Bits of the largest `sound speed + speed` of any particle.
    fastest_bits: atomic<u32>,
    // Unused.
    reserved: u32,
    // Particles that left the grid.
    lost: atomic<u32>,
    steps: u32,
}

// The material state of a point that does not depend on the number of
// dimensions.
struct Point {
    pressure: f32,
    specific_energy: f32,
    plastic_strain: f32,
    plastic_strain_rate: f32,
    temperature: f32,
    damage: f32,
    sound_speed: f32,
}

// Index of an invocation of a pass that runs once per particle, per node or
// per row. Such a pass is dispatched as rows of workgroups, since a single
// row cannot hold more than 65 535 of them.
fn linear_index(id: vec3<u32>, groups: vec3<u32>) -> u32 {
    return id.x + id.y * groups.x * LINEAR_GROUP;
}

// Quadratic B-spline weights of the three stencil nodes for a particle at
// `fraction` cells from the first of them.
fn weights(fraction: f32) -> vec3<f32> {
    return vec3<f32>(
        0.5 * (1.5 - fraction) * (1.5 - fraction),
        0.75 - (fraction - 1.0) * (fraction - 1.0),
        0.5 * (fraction - 0.5) * (fraction - 0.5),
    );
}

// `base` to the power `exponent` for a base that may be zero or, through
// rounding, slightly negative.
fn power(base: f32, exponent: f32) -> f32 {
    if exponent == 0.0 {
        return 1.0;
    }
    if base <= 0.0 {
        return 0.0;
    }
    return pow(base, exponent);
}

// ------------------------------------------------------ equation of state

// Energy-independent part of the Mie-Grüneisen pressure and its derivative
// with respect to the compression mu.
fn cold_pressure(material: Material, mu: f32) -> vec2<f32> {
    let bulk_modulus = material.density * material.sound_speed * material.sound_speed;
    if mu <= 0.0 {
        return vec2<f32>(bulk_modulus * mu, bulk_modulus);
    }
    let a = material.hugoniot_slope - 1.0;
    let b = 1.0 - material.gruneisen_gamma / 2.0;
    let denominator = 1.0 - a * mu;
    let numerator = mu * (1.0 + b * mu);
    let pressure = bulk_modulus * numerator / (denominator * denominator);
    let derivative = bulk_modulus * ((1.0 + 2.0 * b * mu) * denominator + 2.0 * a * numerator)
        / (denominator * denominator * denominator);
    return vec2<f32>(pressure, derivative);
}

// Pressure and bulk sound speed.
fn eos_state(material: Material, density: f32, specific_energy: f32) -> vec2<f32> {
    let mu = density / material.density - 1.0;
    if material.eos_model == EOS_POLYNOMIAL {
        var pressure = material.k1 * mu;
        var derivative = material.k1;
        if mu > 0.0 {
            pressure = mu * (material.k1 + mu * (material.k2 + mu * material.k3));
            derivative = material.k1 + mu * (2.0 * material.k2 + 3.0 * mu * material.k3);
        }
        return vec2<f32>(pressure, sqrt(max(derivative / material.density, 0.0)));
    }
    let cold = cold_pressure(material, mu);
    let energy_derivative = material.gruneisen_gamma * material.density;
    let pressure = cold.x + energy_derivative * specific_energy;
    let squared = cold.y / material.density + pressure * energy_derivative / (density * density);
    return vec2<f32>(pressure, sqrt(max(squared, 0.0)));
}

// ---------------------------------------------------- strength and damage

fn log_rate(strain_rate: f32, reference: f32) -> f32 {
    return log(max(strain_rate / reference, 1.0));
}

fn homologous(temperature: f32, reference: f32, melting: f32) -> f32 {
    return clamp((temperature - reference) / (melting - reference), 0.0, 1.0);
}

fn flow_stress(material: Material, point: Point) -> f32 {
    if material.strength_model == STRENGTH_JOHNSON_HOLMQUIST {
        let pressure = point.pressure / material.hel_pressure;
        let tension = material.tensile_strength / material.hel_pressure;
        let intact = material.intact_coefficient
            * power(pressure + tension, material.intact_exponent);
        let fractured = min(
            material.fractured_coefficient * power(pressure, material.fractured_exponent),
            material.fractured_limit,
        );
        let rate = 1.0 + material.brittle_rate_coefficient
            * log_rate(point.plastic_strain_rate, material.brittle_rate);
        // Fracture can only weaken the material.
        let current = intact - clamp(point.damage, 0.0, 1.0) * (intact - min(fractured, intact));
        return max(current * rate * material.hel_stress, 0.0);
    }
    let hardening = material.yield_stress
        + material.hardening_modulus * power(point.plastic_strain, material.hardening_exponent);
    let rate = 1.0
        + material.rate_coefficient * log_rate(point.plastic_strain_rate, material.strength_rate);
    let softening = 1.0 - power(
        homologous(point.temperature, material.strength_temperature, material.strength_melting),
        material.thermal_exponent,
    );
    return hardening * rate * softening;
}

fn failure_strain(material: Material, triaxiality: f32, point: Point) -> f32 {
    if material.damage_model == DAMAGE_JOHNSON_HOLMQUIST {
        let confinement = (point.pressure + material.damage_tensile_strength)
            / material.damage_hel_pressure;
        return max(material.brittle_d1 * power(confinement, material.brittle_d2), MIN_FAILURE_STRAIN);
    }
    let stress = material.d1 + material.d2 * exp(material.d3 * triaxiality);
    let rate = 1.0 + material.d4 * log_rate(point.plastic_strain_rate, material.damage_rate);
    let heat = 1.0 + material.d5
        * homologous(point.temperature, material.damage_temperature, material.damage_melting);
    return max(stress * rate * heat, MIN_FAILURE_STRAIN);
}

// Whether a point with this damage has no shear strength at all. Failed
// metal flows freely; fractured ceramic keeps a reduced, pressure-dependent
// strength that its strength model accounts for.
fn flows_freely(material: Material, damage: f32) -> bool {
    return damage >= 1.0 && material.strength_model != STRENGTH_JOHNSON_HOLMQUIST;
}

// Radial return of a trial deviatoric stress whose von Mises magnitude is
// `equivalent`: accumulates plastic strain, damage and heat, and returns
// the factor that brings the trial stress back to the yield surface.
fn yield_point(
    material: Material,
    point: ptr<function, Point>,
    equivalent: f32,
    density: f32,
    time_step: f32,
) -> f32 {
    let before = *point;
    let yield_stress = flow_stress(material, before);
    if !(equivalent > yield_stress) {
        (*point).plastic_strain_rate = 0.0;
        return 1.0;
    }
    let strain_increment = (equivalent - yield_stress) / (3.0 * material.shear_modulus);
    let triaxiality = clamp(-before.pressure / max(yield_stress, 1.0), -10.0, 1.5);
    let limit = failure_strain(material, triaxiality, before);
    (*point).damage = min(before.damage + strain_increment / limit, 1.0);
    (*point).temperature = before.temperature
        + material.taylor_quinney * yield_stress * strain_increment
            / (density * material.specific_heat);
    (*point).plastic_strain = before.plastic_strain + strain_increment;
    (*point).plastic_strain_rate = strain_increment / time_step;
    return yield_stress / equivalent;
}

// Pressure from the equation of state, spall and the wave speed at the end
// of a step. Returns whether the point keeps no deviatoric stress.
fn close_point(material: Material, point: ptr<function, Point>, density: f32) -> bool {
    let state = eos_state(material, density, (*point).specific_energy);
    var pressure = state.x;
    if pressure < -material.spall_strength {
        (*point).damage = 1.0;
    }
    let failed = (*point).damage >= 1.0;
    let fluid = flows_freely(material, (*point).damage);
    if failed {
        pressure = max(pressure, 0.0);
    }
    (*point).pressure = pressure;
    // Failed material without confinement is loose: metal always, fractured
    // ceramic once its pressure is gone. It has no shear stiffness to carry
    // a faster wave.
    if failed && (fluid || pressure <= 0.0) {
        (*point).sound_speed = state.y;
    } else {
        (*point).sound_speed =
            sqrt(state.y * state.y + 4.0 * material.shear_modulus / (3.0 * density));
    }
    return fluid;
}

// ---------------------------------------------------------------- contact

// Distance, in cells, between the kernel-weighted centres of mass of two
// bodies that touch along a plane, when the smaller body holds `share_in`
// of the mass at a node. See `rap_solver::contact` for the derivation.
fn touching_separation(share_in: f32) -> f32 {
    let share = min(max(share_in, 1.0e-9), 0.5);
    var moment = 0.0;
    if share <= 1.0 / 6.0 {
        let depth = pow(6.0 * share, 1.0 / 3.0);
        moment = 0.25 * depth * depth * depth - 0.125 * depth * depth * depth * depth;
    } else {
        let missing = 0.5 - share;
        var distance = missing / 0.75;
        for (var step = 0; step < 4; step++) {
            distance -= (distance * distance * distance / 3.0 - 0.75 * distance + missing)
                / (distance * distance - 0.75);
        }
        let squared = distance * distance;
        moment = 0.203125 - 0.375 * squared + 0.25 * squared * squared;
    }
    return moment / (share * (1.0 - share));
}

// Whether two bodies whose centres are `separation` cells apart along the
// contact normal touch.
fn touches(separation: f32, share: f32) -> bool {
    return separation <= touching_separation(min(share, 1.0 - share)) + CONTACT_MARGIN;
}

// Artificial bulk-viscosity pressure of a point being compressed.
fn viscous_pressure(density: f32, divergence: f32, sound_speed: f32) -> f32 {
    let cell = params.cell_size;
    let compression_rate = max(-divergence, 0.0);
    return density * cell * compression_rate
        * (params.linear_viscosity * sound_speed
            + params.quadratic_viscosity * cell * compression_rate);
}

// ------------------------------------------------------- shared time step

@compute @workgroup_size(1)
fn prepare() {
    let remaining = clock.end_time - clock.time;
    if remaining <= 0.0 {
        clock.time_step = 0.0;
        return;
    }
    let fastest = bitcast<f32>(atomicLoad(&clock.fastest_bits));
    atomicStore(&clock.fastest_bits, 0u);
    var time_step = remaining;
    if fastest > 0.0 {
        time_step = min(remaining, params.cfl * params.cell_size / fastest);
    }
    clock.time_step = time_step;
}

@compute @workgroup_size(1)
fn finish() {
    let time_step = clock.time_step;
    if time_step <= 0.0 {
        return;
    }
    if time_step >= clock.end_time - clock.time {
        // The step was cut to end exactly at the requested time.
        clock.time = clock.end_time;
        clock.compensation = 0.0;
    } else {
        // Kahan summation keeps the clock accurate over many small steps.
        let corrected = time_step - clock.compensation;
        let advanced = clock.time + corrected;
        clock.compensation = (advanced - clock.time) - corrected;
        clock.time = advanced;
    }
    clock.steps += 1u;
}
