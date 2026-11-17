// Explicit MLS-MPM in two dimensions (plane strain or axisymmetric).
//
// The scheme is the one of the CPU reference solver; see its documentation
// for the physics. What differs is the particle-to-grid transfer: floats
// cannot be accumulated atomically, so particles are first sorted by the
// cell of their interpolation stencil (a counting sort: count, prefix sum,
// fill) and every grid node then gathers from the cells that can reach it.
//
// One time step is the sequence of entry points
//   prepare, clear_bins, count_particles, row_prefix, row_offsets,
//   fill_bins, grid, particles_update, finish.
// `prepare`, `finish` and the material models are in `common.wgsl`.

const MIN_RADIUS_CELLS: f32 = 0.1;

struct Params {
    nodes: vec2<u32>,
    bodies: u32,
    particle_count: u32,
    interior_min: vec2<f32>,
    interior_max: vec2<f32>,
    cell_size: f32,
    cfl: f32,
    linear_viscosity: f32,
    quadratic_viscosity: f32,
    friction: f32,
    axis: f32,
    axisymmetric: u32,
    wall_left: u32,
    wall_right: u32,
    wall_bottom: u32,
    wall_top: u32,
    padding: u32,
}

struct Particle {
    position: vec2<f32>,
    velocity: vec2<f32>,
    // Velocity gradient stored by columns: (xx, yx, xy, yy).
    gradient: vec4<f32>,
    mass: f32,
    volume: f32,
    pressure: f32,
    viscous_pressure: f32,
    // Deviatoric stress (xx, yy, xy); zz follows from the zero trace.
    deviator: vec3<f32>,
    specific_energy: f32,
    plastic_strain: f32,
    plastic_strain_rate: f32,
    temperature: f32,
    damage: f32,
    sound_speed: f32,
    body: u32,
    material: u32,
    status: u32,
}

struct NodeState {
    // Velocity at the end of the step.
    velocity: vec2<f32>,
    // Time-centred velocity, used for the stress work.
    work_velocity: vec2<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> materials: array<Material>;
@group(0) @binding(2) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(3) var<storage, read_write> nodes: array<NodeState>;
@group(0) @binding(4) var<storage, read_write> node_heat: array<f32>;
// Number of particles in each bin; doubles as the fill cursor.
@group(0) @binding(5) var<storage, read_write> bin_count: array<atomic<u32>>;
// For each bin, the number of particles in earlier bins of its row; then,
// for each row, the number of particles in earlier rows. Their sum is the
// index in `bin_items` of the first particle of a bin.
@group(0) @binding(6) var<storage, read_write> bin_start: array<u32>;
// Particle indices, grouped by bin.
@group(0) @binding(7) var<storage, read_write> bin_items: array<u32>;
@group(0) @binding(8) var<storage, read_write> clock: Clock;

// ---------------------------------------------------------------- helpers

// Index of the first stencil node of a particle; valid only if `in_grid`.
fn stencil_base(position: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(floor(position / params.cell_size - 0.5));
}

fn in_grid(position: vec2<f32>) -> bool {
    let scaled = position / params.cell_size - 0.5;
    let upper = vec2<f32>(params.nodes) - 2.0;
    // Written so that a NaN position fails the test.
    return scaled.x >= 0.0 && scaled.y >= 0.0 && scaled.x < upper.x && scaled.y < upper.y;
}

// Product of a velocity gradient stored by columns with a vector.
fn apply_gradient(gradient: vec4<f32>, arm: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        gradient.x * arm.x + gradient.z * arm.y,
        gradient.y * arm.x + gradient.w * arm.y,
    );
}

// Index in `bin_items` of the first particle of the bin at `cell`.
fn bin_first(cell: vec2<u32>) -> u32 {
    let row_table = params.nodes.x * params.nodes.y;
    return bin_start[cell.x + cell.y * params.nodes.x] + bin_start[row_table + cell.y];
}

fn radius_of(position: vec2<f32>) -> f32 {
    return max(position.y - params.axis, MIN_RADIUS_CELLS * params.cell_size);
}

fn equivalent_stress(deviator: vec3<f32>) -> f32 {
    let zz = -(deviator.x + deviator.y);
    let squares = deviator.x * deviator.x + deviator.y * deviator.y + zz * zz;
    return sqrt(1.5 * (squares + 2.0 * deviator.z * deviator.z));
}

// ------------------------------------------------------- grid constraints

fn apply_wall(velocity: vec2<f32>, wall: u32, inward: vec2<f32>) -> vec2<f32> {
    let normal_speed = dot(velocity, inward);
    if wall == WALL_STICKY {
        return vec2<f32>(0.0);
    }
    if wall == WALL_SLIDING {
        return velocity - normal_speed * inward;
    }
    if wall == WALL_FRICTIONLESS {
        return velocity - min(normal_speed, 0.0) * inward;
    }
    return velocity;
}

fn apply_walls(velocity_in: vec2<f32>, node: vec2<u32>) -> vec2<f32> {
    var velocity = velocity_in;
    if node.x <= WALL_CELLS {
        velocity = apply_wall(velocity, params.wall_left, vec2<f32>(1.0, 0.0));
    }
    if node.x + WALL_CELLS >= params.nodes.x - 1u {
        velocity = apply_wall(velocity, params.wall_right, vec2<f32>(-1.0, 0.0));
    }
    if node.y <= WALL_CELLS {
        velocity = apply_wall(velocity, params.wall_bottom, vec2<f32>(0.0, 1.0));
    }
    if node.y + WALL_CELLS >= params.nodes.y - 1u {
        velocity = apply_wall(velocity, params.wall_top, vec2<f32>(0.0, -1.0));
    }
    return velocity;
}

// Contact of one body with the others at a node; see the CPU solver.
fn resolve_contact(velocity: vec2<f32>, centre: vec2<f32>, outward: vec2<f32>) -> vec2<f32> {
    let reach = length(outward);
    if !(reach > 0.0) {
        return velocity;
    }
    let normal = outward / reach;
    let relative = velocity - centre;
    let approach = dot(relative, normal);
    if approach <= 0.0 {
        return velocity;
    }
    let tangential = relative - approach * normal;
    let slip = length(tangential);
    var friction_change = vec2<f32>(0.0);
    if slip > 0.0 {
        friction_change = tangential * (min(params.friction * approach, slip) / slip);
    }
    return velocity - approach * normal - friction_change;
}

// ------------------------------------------------------------ entry points

@compute @workgroup_size(64)
fn clear_bins(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let bin = linear_index(id, groups);
    if bin < params.nodes.x * params.nodes.y {
        atomicStore(&bin_count[bin], 0u);
    }
}

// Retires particles that crossed an outflow wall, flags those that left
// the grid and counts the rest into their bins.
@compute @workgroup_size(64)
fn count_particles(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let index = linear_index(id, groups);
    if index >= params.particle_count || clock.time_step <= 0.0 {
        return;
    }
    if particles[index].status != ACTIVE {
        return;
    }
    let position = particles[index].position;
    let gone = (params.wall_left == WALL_OUTFLOW && position.x < params.interior_min.x)
        || (params.wall_right == WALL_OUTFLOW && position.x > params.interior_max.x)
        || (params.wall_bottom == WALL_OUTFLOW && position.y < params.interior_min.y)
        || (params.wall_top == WALL_OUTFLOW && position.y > params.interior_max.y);
    if gone {
        particles[index].status = DEPARTED;
        return;
    }
    if !in_grid(position) {
        particles[index].status = LOST;
        atomicAdd(&clock.lost, 1u);
        return;
    }
    let base = stencil_base(position);
    atomicAdd(&bin_count[u32(base.x) + u32(base.y) * params.nodes.x], 1u);
}

// The prefix sum over all bins is split in two so that no invocation loops
// over more than one row or one column of the grid: first within each row,
// in parallel, then over the row totals.

// Prefix sum within one row. Resets the counts, which the fill pass uses as
// cursors, and leaves the row total in the row table.
@compute @workgroup_size(64)
fn row_prefix(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let row = linear_index(id, groups);
    if row >= params.nodes.y || clock.time_step <= 0.0 {
        return;
    }
    let first = row * params.nodes.x;
    var total = 0u;
    for (var column = 0u; column < params.nodes.x; column++) {
        bin_start[first + column] = total;
        total += atomicExchange(&bin_count[first + column], 0u);
    }
    bin_start[params.nodes.x * params.nodes.y + row] = total;
}

// Turns the row totals into the number of particles in earlier rows.
@compute @workgroup_size(1)
fn row_offsets() {
    if clock.time_step <= 0.0 {
        return;
    }
    let row_table = params.nodes.x * params.nodes.y;
    var total = 0u;
    for (var row = 0u; row < params.nodes.y; row++) {
        let in_row = bin_start[row_table + row];
        bin_start[row_table + row] = total;
        total += in_row;
    }
}

@compute @workgroup_size(64)
fn fill_bins(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let index = linear_index(id, groups);
    if index >= params.particle_count || clock.time_step <= 0.0 {
        return;
    }
    if particles[index].status != ACTIVE {
        return;
    }
    let base = stencil_base(particles[index].position);
    let bin = u32(base.x) + u32(base.y) * params.nodes.x;
    let slot = bin_first(vec2<u32>(base)) + atomicAdd(&bin_count[bin], 1u);
    bin_items[slot] = index;
}

@compute @workgroup_size(8, 8)
fn grid(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.nodes.x || id.y >= params.nodes.y {
        return;
    }
    let time_step = clock.time_step;
    if time_step <= 0.0 {
        return;
    }
    let node_count = params.nodes.x * params.nodes.y;
    let node = id.x + id.y * params.nodes.x;
    let cell = params.cell_size;
    let gradient_scale = 4.0 / (cell * cell);

    // Gather mass, momentum, stress impulse and surface normal per body.
    var mass: array<f32, 8>;
    var momentum: array<vec2<f32>, 8>;
    var impulse: array<vec2<f32>, 8>;
    var outward: array<vec2<f32>, 8>;
    let here = vec2<i32>(id.xy);
    let last_base = vec2<i32>(params.nodes) - 3;
    for (var oy = 0; oy < 3; oy++) {
        for (var ox = 0; ox < 3; ox++) {
            let base = here - vec2<i32>(ox, oy);
            if base.x < 0 || base.y < 0 || base.x > last_base.x || base.y > last_base.y {
                continue;
            }
            let bin = u32(base.x) + u32(base.y) * params.nodes.x;
            let first = bin_first(vec2<u32>(base));
            let count = atomicLoad(&bin_count[bin]);
            for (var item = first; item < first + count; item++) {
                let particle = particles[bin_items[item]];
                let fraction = particle.position / cell - vec2<f32>(base);
                let weight = weights(fraction.x)[ox] * weights(fraction.y)[oy];
                let arm = (vec2<f32>(f32(ox), f32(oy)) - fraction) * cell;
                let normal = particle.pressure + particle.viscous_pressure;
                let deviator = particle.deviator;
                let traction = vec2<f32>(
                    (deviator.x - normal) * arm.x + deviator.z * arm.y,
                    deviator.z * arm.x + (deviator.y - normal) * arm.y,
                );
                // MLS-MPM nodal force: f = -V (4/dx^2) w sigma (x_i - x_p).
                var force = (-time_step * particle.volume * gradient_scale) * traction;
                if params.axisymmetric != 0u {
                    let hoop = -(deviator.x + deviator.y) - normal;
                    force.y -= time_step * particle.volume * hoop / radius_of(particle.position);
                }
                let sent = particle.velocity + apply_gradient(particle.gradient, arm);
                let body = particle.body;
                mass[body] += weight * particle.mass;
                momentum[body] += weight * particle.mass * sent;
                impulse[body] += weight * force;
                outward[body] += weight * particle.mass * arm;
            }
        }
    }

    // Free motion of every body and the mean motion of them all.
    var before: array<vec2<f32>, 8>;
    var unconstrained: array<vec2<f32>, 8>;
    var total_mass = 0.0;
    var total_momentum = vec2<f32>(0.0);
    var total_outward = vec2<f32>(0.0);
    for (var body = 0u; body < params.bodies; body++) {
        if mass[body] > 0.0 {
            before[body] = momentum[body] / mass[body];
            unconstrained[body] = before[body] + impulse[body] / mass[body];
            total_mass += mass[body];
            total_momentum += mass[body] * unconstrained[body];
            total_outward += outward[body];
        }
    }

    var dissipated = 0.0;
    for (var body = 0u; body < params.bodies; body++) {
        var velocity = vec2<f32>(0.0);
        var work_velocity = vec2<f32>(0.0);
        if mass[body] > 0.0 {
            velocity = unconstrained[body];
            if mass[body] < total_mass {
                let centre = total_momentum / total_mass;
                let normal = 2.0 * outward[body] - total_outward;
                // From each centre of mass to the node.
                let reach = outward[body] / mass[body]
                    - (total_outward - outward[body]) / (total_mass - mass[body]);
                let span = length(normal);
                if span > 0.0
                    && touches(dot(reach, normal) / (span * cell), mass[body] / total_mass)
                {
                    velocity = resolve_contact(velocity, centre, normal);
                }
            }
            velocity = apply_walls(velocity, id.xy);
            dissipated += 0.5 * mass[body]
                * dot(unconstrained[body] - velocity, before[body] + velocity);
            work_velocity = 0.5 * (before[body] + velocity);
        }
        nodes[body * node_count + node] = NodeState(velocity, work_velocity);
    }
    if total_mass > 0.0 {
        node_heat[node] = dissipated / total_mass;
    } else {
        node_heat[node] = 0.0;
    }
}

@compute @workgroup_size(64)
fn particles_update(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let index = linear_index(id, groups);
    let time_step = clock.time_step;
    if index >= params.particle_count || time_step <= 0.0 {
        return;
    }
    var particle = particles[index];
    if particle.status != ACTIVE {
        return;
    }
    let cell = params.cell_size;
    let gradient_scale = 4.0 / (cell * cell);
    let node_count = params.nodes.x * params.nodes.y;
    let base = stencil_base(particle.position);
    let fraction = particle.position / cell - vec2<f32>(base);
    let weights_x = weights(fraction.x);
    let weights_y = weights(fraction.y);

    // Grid to particle: velocity, velocity gradient and energy bookkeeping.
    var velocity = vec2<f32>(0.0);
    var work_velocity = vec2<f32>(0.0);
    var column_x = vec2<f32>(0.0);
    var column_y = vec2<f32>(0.0);
    var work_column_x = vec2<f32>(0.0);
    var work_column_y = vec2<f32>(0.0);
    var mixing_loss = 0.0;
    var grid_energy = 0.0;
    var constraint_heat = 0.0;
    for (var oy = 0; oy < 3; oy++) {
        for (var ox = 0; ox < 3; ox++) {
            let node = u32(base.x + ox) + u32(base.y + oy) * params.nodes.x;
            let state = nodes[particle.body * node_count + node];
            let weight = weights_x[ox] * weights_y[oy];
            let arm = (vec2<f32>(f32(ox), f32(oy)) - fraction) * cell;
            let weighted = weight * state.velocity;
            velocity += weighted;
            column_x += weighted * arm.x;
            column_y += weighted * arm.y;
            let work = weight * state.work_velocity;
            work_velocity += work;
            work_column_x += work * arm.x;
            work_column_y += work * arm.y;
            // Node velocity before forces and constraints acted.
            let before = 2.0 * state.work_velocity - state.velocity;
            let sent = particle.velocity + apply_gradient(particle.gradient, arm);
            mixing_loss += 0.5 * weight * dot(sent - before, sent - before);
            grid_energy += 0.5 * weight * dot(state.velocity, state.velocity);
            constraint_heat += weight * node_heat[node];
        }
    }
    column_x *= gradient_scale;
    column_y *= gradient_scale;
    work_column_x *= gradient_scale;
    work_column_y *= gradient_scale;

    // Kinetic energy the transfers cannot represent returns as heat.
    let gradient_norm = dot(column_x, column_x) + dot(column_y, column_y);
    let affine_energy = 0.5 * (dot(velocity, velocity) + gradient_norm / gradient_scale);
    let projection_loss = max(grid_energy - affine_energy, 0.0);
    particle.specific_energy += mixing_loss + projection_loss + constraint_heat;

    var hoop_rate = 0.0;
    var work_hoop_rate = 0.0;
    if params.axisymmetric != 0u {
        let radius = radius_of(particle.position);
        hoop_rate = velocity.y / radius;
        work_hoop_rate = work_velocity.y / radius;
    }
    particle.velocity = velocity;
    particle.gradient = vec4<f32>(column_x, column_y);
    particle.position += time_step * velocity;

    // Constitutive update.
    let material = materials[particle.material];
    let l_xx = column_x.x;
    let l_yx = column_x.y;
    let l_xy = column_y.x;
    let l_yy = column_y.y;
    let rate_xy = 0.5 * (l_xy + l_yx);
    let spin = 0.5 * (l_xy - l_yx);
    let divergence = l_xx + l_yy + hoop_rate;
    let mean_rate = divergence / 3.0;

    let old = particle.deviator;
    let old_zz = -(old.x + old.y);
    let normal = particle.pressure + particle.viscous_pressure;
    let stress_power = (old.x - normal) * work_column_x.x
        + (old.y - normal) * work_column_y.y
        + old.z * (work_column_y.x + work_column_x.y)
        + (old_zz - normal) * work_hoop_rate;
    particle.specific_energy += time_step * stress_power * particle.volume / particle.mass;

    particle.volume *= exp(time_step * divergence);
    let density = particle.mass / particle.volume;

    var point = Point(
        particle.pressure,
        particle.specific_energy,
        particle.plastic_strain,
        particle.plastic_strain_rate,
        particle.temperature,
        particle.damage,
        particle.sound_speed,
    );
    if flows_freely(material, point.damage) {
        point.plastic_strain_rate = 0.0;
    } else {
        // Trial elastic deviator in the Jaumann rate, then radial return.
        let shear = material.shear_modulus;
        let trial = vec3<f32>(
            old.x + time_step * (2.0 * shear * (l_xx - mean_rate) + 2.0 * spin * old.z),
            old.y + time_step * (2.0 * shear * (l_yy - mean_rate) - 2.0 * spin * old.z),
            old.z + time_step * (2.0 * shear * rate_xy + spin * (old.y - old.x)),
        );
        particle.deviator =
            trial * yield_point(material, &point, equivalent_stress(trial), density, time_step);
    }
    if close_point(material, &point, density) {
        particle.deviator = vec3<f32>(0.0);
    }
    particle.pressure = point.pressure;
    particle.plastic_strain = point.plastic_strain;
    particle.plastic_strain_rate = point.plastic_strain_rate;
    particle.temperature = point.temperature;
    particle.damage = point.damage;
    particle.sound_speed = point.sound_speed;
    particle.viscous_pressure = viscous_pressure(density, divergence, point.sound_speed);

    particles[index] = particle;
    // Positive floats order like their bit patterns.
    atomicMax(&clock.fastest_bits, bitcast<u32>(particle.sound_speed + length(particle.velocity)));
}
