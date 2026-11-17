// Explicit MLS-MPM in three dimensions.
//
// The scheme is the one of `flat.wgsl` with a 3 x 3 x 3 interpolation
// stencil and a full deviatoric stress tensor; see the CPU reference solver
// for the physics. Particles are sorted by the cell of their stencil and
// every grid node gathers from the 27 cells that can reach it.
//
// One time step is the sequence of entry points
//   prepare, clear_bins, count_particles, row_prefix, row_offsets,
//   fill_bins, grid, particles_update, finish.
// `prepare`, `finish` and the material models are in `common.wgsl`.

struct Params {
    nodes: vec3<u32>,
    bodies: u32,
    interior_min: vec3<f32>,
    particle_count: u32,
    interior_max: vec3<f32>,
    cell_size: f32,
    cfl: f32,
    linear_viscosity: f32,
    quadratic_viscosity: f32,
    friction: f32,
    // Walls at the low and the high side of x, y and z.
    walls_low: vec3<u32>,
    walls_high: vec3<u32>,
}

// Scalars fill the word left after every three-component vector.
struct Particle {
    position: vec3<f32>,
    mass: f32,
    velocity: vec3<f32>,
    volume: f32,
    // Columns of the velocity gradient: derivatives along x, y and z.
    gradient_x: vec3<f32>,
    pressure: f32,
    gradient_y: vec3<f32>,
    viscous_pressure: f32,
    gradient_z: vec3<f32>,
    specific_energy: f32,
    // Deviatoric stress: (xx, yy, zz) and (xy, xz, yz).
    deviator_normal: vec3<f32>,
    plastic_strain: f32,
    deviator_shear: vec3<f32>,
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
    velocity: vec3<f32>,
    // Time-centred velocity, used for the stress work.
    work_velocity: vec3<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> materials: array<Material>;
@group(0) @binding(2) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(3) var<storage, read_write> nodes: array<NodeState>;
@group(0) @binding(4) var<storage, read_write> node_heat: array<f32>;
// Number of particles in each bin; doubles as the fill cursor.
@group(0) @binding(5) var<storage, read_write> bin_count: array<atomic<u32>>;
// For each bin, the number of particles in earlier bins of its row along x;
// then, for each row, the number of particles in earlier rows. Their sum is
// the index in `bin_items` of the first particle of a bin.
@group(0) @binding(6) var<storage, read_write> bin_start: array<u32>;
// Particle indices, grouped by bin.
@group(0) @binding(7) var<storage, read_write> bin_items: array<u32>;
@group(0) @binding(8) var<storage, read_write> clock: Clock;

// ---------------------------------------------------------------- helpers

fn node_total() -> u32 {
    return params.nodes.x * params.nodes.y * params.nodes.z;
}

fn row_total() -> u32 {
    return params.nodes.y * params.nodes.z;
}

// Index of the node, or of the bin, at `cell`.
fn flat_index(cell: vec3<u32>) -> u32 {
    return cell.x + (cell.y + cell.z * params.nodes.y) * params.nodes.x;
}

// Index of the first stencil node of a particle; valid only if `in_grid`.
fn stencil_base(position: vec3<f32>) -> vec3<i32> {
    return vec3<i32>(floor(position / params.cell_size - 0.5));
}

fn in_grid(position: vec3<f32>) -> bool {
    let scaled = position / params.cell_size - 0.5;
    let upper = vec3<f32>(params.nodes) - 2.0;
    // Written so that a NaN position fails the test.
    return scaled.x >= 0.0 && scaled.y >= 0.0 && scaled.z >= 0.0
        && scaled.x < upper.x && scaled.y < upper.y && scaled.z < upper.z;
}

// Index in `bin_items` of the first particle of the bin at `cell`.
fn bin_first(cell: vec3<u32>) -> u32 {
    let row = cell.y + cell.z * params.nodes.y;
    return bin_start[flat_index(cell)] + bin_start[node_total() + row];
}

// A symmetric tensor given by its normal and shear components, times a
// vector.
fn apply_symmetric(normal: vec3<f32>, shear: vec3<f32>, arm: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        normal.x * arm.x + shear.x * arm.y + shear.y * arm.z,
        shear.x * arm.x + normal.y * arm.y + shear.z * arm.z,
        shear.y * arm.x + shear.z * arm.y + normal.z * arm.z,
    );
}

fn symmetric(normal: vec3<f32>, shear: vec3<f32>) -> mat3x3<f32> {
    return mat3x3<f32>(
        vec3<f32>(normal.x, shear.x, shear.y),
        vec3<f32>(shear.x, normal.y, shear.z),
        vec3<f32>(shear.y, shear.z, normal.z),
    );
}

// Largest principal value of a traceless symmetric tensor given by its
// normal components (xx, yy, zz) and its shear components (xy, xz, yz).
fn largest_principal(normal: vec3<f32>, shear: vec3<f32>) -> f32 {
    // Half the sum of squares and the determinant.
    let second = 0.5 * dot(normal, normal) + dot(shear, shear);
    if second <= 0.0 {
        return 0.0;
    }
    let third = normal.x * normal.y * normal.z + 2.0 * shear.x * shear.y * shear.z
        - normal.x * shear.z * shear.z - normal.y * shear.y * shear.y
        - normal.z * shear.x * shear.x;
    let angle = clamp(1.5 * sqrt(3.0) * third / (second * sqrt(second)), -1.0, 1.0);
    return 2.0 * sqrt(second / 3.0) * cos(acos(angle) / 3.0);
}

// Sum of the products of corresponding components of two tensors.
fn inner(a: mat3x3<f32>, b: mat3x3<f32>) -> f32 {
    return dot(a[0], b[0]) + dot(a[1], b[1]) + dot(a[2], b[2]);
}

fn trace(matrix: mat3x3<f32>) -> f32 {
    return matrix[0].x + matrix[1].y + matrix[2].z;
}

// ------------------------------------------------------- grid constraints

fn apply_wall(velocity: vec3<f32>, wall: u32, inward: vec3<f32>) -> vec3<f32> {
    let normal_speed = dot(velocity, inward);
    if wall == WALL_STICKY {
        return vec3<f32>(0.0);
    }
    if wall == WALL_SLIDING {
        return velocity - normal_speed * inward;
    }
    if wall == WALL_FRICTIONLESS {
        return velocity - min(normal_speed, 0.0) * inward;
    }
    return velocity;
}

fn apply_walls(velocity_in: vec3<f32>, node: vec3<u32>) -> vec3<f32> {
    var velocity = velocity_in;
    for (var axis = 0u; axis < 3u; axis++) {
        var inward = vec3<f32>(0.0);
        inward[axis] = 1.0;
        if node[axis] <= WALL_CELLS {
            velocity = apply_wall(velocity, params.walls_low[axis], inward);
        }
        if node[axis] + WALL_CELLS >= params.nodes[axis] - 1u {
            velocity = apply_wall(velocity, params.walls_high[axis], -inward);
        }
    }
    return velocity;
}

// Contact of one body with the others at a node; see the CPU solver.
fn resolve_contact(velocity: vec3<f32>, centre: vec3<f32>, outward: vec3<f32>) -> vec3<f32> {
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
    var friction_change = vec3<f32>(0.0);
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
    if bin < node_total() {
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
    var gone = false;
    for (var axis = 0u; axis < 3u; axis++) {
        gone = gone
            || (params.walls_low[axis] == WALL_OUTFLOW
                && position[axis] < params.interior_min[axis])
            || (params.walls_high[axis] == WALL_OUTFLOW
                && position[axis] > params.interior_max[axis]);
    }
    if gone {
        particles[index].status = DEPARTED;
        return;
    }
    if !in_grid(position) {
        particles[index].status = LOST;
        atomicAdd(&clock.lost, 1u);
        return;
    }
    atomicAdd(&bin_count[flat_index(vec3<u32>(stencil_base(position)))], 1u);
}

// The prefix sum over all bins is split in two so that no invocation loops
// over more than one row of the grid or over more than the rows: first
// within each row along x, in parallel, then over the row totals.

// Prefix sum within one row. Resets the counts, which the fill pass uses as
// cursors, and leaves the row total in the row table.
@compute @workgroup_size(64)
fn row_prefix(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let row = linear_index(id, groups);
    if row >= row_total() || clock.time_step <= 0.0 {
        return;
    }
    let first = row * params.nodes.x;
    var total = 0u;
    for (var column = 0u; column < params.nodes.x; column++) {
        bin_start[first + column] = total;
        total += atomicExchange(&bin_count[first + column], 0u);
    }
    bin_start[node_total() + row] = total;
}

// Turns the row totals into the number of particles in earlier rows.
@compute @workgroup_size(1)
fn row_offsets() {
    if clock.time_step <= 0.0 {
        return;
    }
    let row_table = node_total();
    let rows = row_total();
    var total = 0u;
    for (var row = 0u; row < rows; row++) {
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
    let cell = vec3<u32>(stencil_base(particles[index].position));
    let slot = bin_first(cell) + atomicAdd(&bin_count[flat_index(cell)], 1u);
    bin_items[slot] = index;
}

@compute @workgroup_size(4, 4, 4)
fn grid(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.nodes.x || id.y >= params.nodes.y || id.z >= params.nodes.z {
        return;
    }
    let time_step = clock.time_step;
    if time_step <= 0.0 {
        return;
    }
    let node_count = node_total();
    let node = flat_index(id);
    let cell = params.cell_size;
    let gradient_scale = 4.0 / (cell * cell);

    // Gather mass, momentum, stress impulse and surface normal per body.
    var mass: array<f32, 8>;
    var momentum: array<vec3<f32>, 8>;
    var impulse: array<vec3<f32>, 8>;
    var outward: array<vec3<f32>, 8>;
    // The bins that reach this node are three neighbours along x in each
    // of nine rows. The sort keeps the bins of a row together, so each row
    // is one run of particles, and an empty run costs three reads.
    let here = vec3<i32>(id);
    let low = max(here - 2, vec3<i32>(0));
    let high = min(here, vec3<i32>(params.nodes) - 3);
    var occupied = false;
    for (var z = low.z; z <= high.z; z++) {
        for (var y = low.y; y <= high.y; y++) {
            if low.x > high.x {
                continue;
            }
            let row = u32(y) + u32(z) * params.nodes.y;
            let row_first = row * params.nodes.x;
            let last_bin = row_first + u32(high.x);
            let run_start = bin_start[row_first + u32(low.x)];
            let run_end = bin_start[last_bin] + atomicLoad(&bin_count[last_bin]);
            if run_end == run_start {
                continue;
            }
            occupied = true;
            let row_offset = bin_start[node_count + row];
            for (var item = row_offset + run_start; item < row_offset + run_end; item++) {
                let particle = particles[bin_items[item]];
                let base = stencil_base(particle.position);
                let offset = here - base;
                let fraction = particle.position / cell - vec3<f32>(base);
                let weight = weights(fraction.x)[offset.x] * weights(fraction.y)[offset.y]
                    * weights(fraction.z)[offset.z];
                let arm = (vec3<f32>(offset) - fraction) * cell;
                let normal = particle.pressure + particle.viscous_pressure;
                let traction = apply_symmetric(
                    particle.deviator_normal - normal,
                    particle.deviator_shear,
                    arm,
                );
                // MLS-MPM nodal force: f = -V (4/dx^2) w sigma (x_i - x_p).
                let force = (-time_step * particle.volume * gradient_scale) * traction;
                let sent = particle.velocity + particle.gradient_x * arm.x
                    + particle.gradient_y * arm.y + particle.gradient_z * arm.z;
                let body = particle.body;
                mass[body] += weight * particle.mass;
                momentum[body] += weight * particle.mass * sent;
                impulse[body] += weight * force;
                outward[body] += weight * particle.mass * arm;
            }
        }
    }
    // No particle reads a node that none reaches, so most of a sparse grid
    // needs nothing written.
    if !occupied {
        return;
    }

    // Free motion of every body and the mean motion of them all.
    var before: array<vec3<f32>, 8>;
    var unconstrained: array<vec3<f32>, 8>;
    var total_mass = 0.0;
    var total_momentum = vec3<f32>(0.0);
    for (var body = 0u; body < params.bodies; body++) {
        if mass[body] > 0.0 {
            before[body] = momentum[body] / mass[body];
            unconstrained[body] = before[body] + impulse[body] / mass[body];
            total_mass += mass[body];
            total_momentum += mass[body] * unconstrained[body];
        }
    }

    var dissipated = 0.0;
    for (var body = 0u; body < params.bodies; body++) {
        var velocity = vec3<f32>(0.0);
        var work_velocity = vec3<f32>(0.0);
        if mass[body] > 0.0 {
            velocity = unconstrained[body];
            // The other bodies are summed one by one, not taken as the
            // total less this one: with two bodies each then sees exactly
            // the other's values, and both reach the same verdict on
            // whether they touch.
            var others_mass = 0.0;
            var others_outward = vec3<f32>(0.0);
            for (var other = 0u; other < params.bodies; other++) {
                if other != body && mass[other] > 0.0 {
                    others_mass += mass[other];
                    others_outward += outward[other];
                }
            }
            if others_mass > 0.0 {
                let centre = total_momentum / total_mass;
                let normal = outward[body] - others_outward;
                // From each centre of mass to the node.
                let reach = outward[body] / mass[body] - others_outward / others_mass;
                let span = length(normal);
                let share = min(mass[body], others_mass) / (mass[body] + others_mass);
                if span > 0.0 && touches(dot(reach, normal) / (span * cell), share) {
                    velocity = resolve_contact(velocity, centre, normal);
                }
            }
            velocity = apply_walls(velocity, id);
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
    let node_count = node_total();
    let base = stencil_base(particle.position);
    let fraction = particle.position / cell - vec3<f32>(base);
    let weights_x = weights(fraction.x);
    let weights_y = weights(fraction.y);
    let weights_z = weights(fraction.z);

    // Grid to particle: velocity, velocity gradient and energy bookkeeping.
    var velocity = vec3<f32>(0.0);
    var gradient = mat3x3<f32>();
    var work_gradient = mat3x3<f32>();
    var mixing_loss = 0.0;
    var grid_energy = 0.0;
    var constraint_heat = 0.0;
    for (var oz = 0; oz < 3; oz++) {
        for (var oy = 0; oy < 3; oy++) {
            for (var ox = 0; ox < 3; ox++) {
                let offset = vec3<i32>(ox, oy, oz);
                let node = flat_index(vec3<u32>(base + offset));
                let state = nodes[particle.body * node_count + node];
                let weight = weights_x[ox] * weights_y[oy] * weights_z[oz];
                let arm = (vec3<f32>(offset) - fraction) * cell;
                let weighted = weight * state.velocity;
                velocity += weighted;
                gradient += mat3x3<f32>(weighted * arm.x, weighted * arm.y, weighted * arm.z);
                let work = weight * state.work_velocity;
                work_gradient += mat3x3<f32>(work * arm.x, work * arm.y, work * arm.z);
                // Node velocity before forces and constraints acted.
                let before = 2.0 * state.work_velocity - state.velocity;
                let sent = particle.velocity + particle.gradient_x * arm.x
                    + particle.gradient_y * arm.y + particle.gradient_z * arm.z;
                mixing_loss += 0.5 * weight * dot(sent - before, sent - before);
                grid_energy += 0.5 * weight * dot(state.velocity, state.velocity);
                constraint_heat += weight * node_heat[node];
            }
        }
    }
    gradient *= gradient_scale;
    work_gradient *= gradient_scale;

    // Kinetic energy the transfers cannot represent returns as heat.
    let affine_energy =
        0.5 * (dot(velocity, velocity) + inner(gradient, gradient) / gradient_scale);
    let projection_loss = max(grid_energy - affine_energy, 0.0);
    particle.specific_energy += mixing_loss + projection_loss + constraint_heat;

    particle.velocity = velocity;
    particle.gradient_x = gradient[0];
    particle.gradient_y = gradient[1];
    particle.gradient_z = gradient[2];
    particle.position += time_step * velocity;

    // Constitutive update.
    let material = materials[particle.material];
    let transposed = transpose(gradient);
    let rate = 0.5 * (gradient + transposed);
    let spin = 0.5 * (gradient - transposed);
    let divergence = trace(gradient);

    let old = symmetric(particle.deviator_normal, particle.deviator_shear);
    let normal = particle.pressure + particle.viscous_pressure;
    let stress_power = inner(old, work_gradient) - normal * trace(work_gradient);
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
        let identity = mat3x3<f32>(
            vec3<f32>(1.0, 0.0, 0.0),
            vec3<f32>(0.0, 1.0, 0.0),
            vec3<f32>(0.0, 0.0, 1.0),
        );
        let deviatoric_rate = rate - (divergence / 3.0) * identity;
        let trial = old
            + time_step * ((2.0 * material.shear_modulus) * deviatoric_rate
                + spin * old - old * spin);
        let equivalent = sqrt(1.5 * inner(trial, trial));
        let kept = trial * yield_point(material, &point, equivalent, density, time_step);
        particle.deviator_normal = vec3<f32>(kept[0].x, kept[1].y, kept[2].z);
        // The trial stress is symmetric up to rounding; keep it exactly so.
        particle.deviator_shear = 0.5 * vec3<f32>(
            kept[1].x + kept[0].y,
            kept[2].x + kept[0].z,
            kept[2].y + kept[1].z,
        );
    }
    let largest = largest_principal(particle.deviator_normal, particle.deviator_shear);
    if close_point(material, &point, density, largest) {
        particle.deviator_normal = vec3<f32>(0.0);
        particle.deviator_shear = vec3<f32>(0.0);
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
