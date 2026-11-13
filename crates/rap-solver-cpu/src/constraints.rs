//! Velocity constraints applied on grid nodes: rigid walls and contact
//! between bodies.

use glam::DVec2;
use rap_solver::{Grid, WALL_CELLS, Wall, Walls};

/// Applies a wall with inward unit normal `inward` to a node velocity.
pub(crate) fn apply_wall(velocity: DVec2, wall: Wall, inward: DVec2) -> DVec2 {
    let normal_speed = velocity.dot(inward);
    match wall {
        Wall::Sticky => DVec2::ZERO,
        Wall::Sliding => velocity - normal_speed * inward,
        Wall::Frictionless => velocity - normal_speed.min(0.0) * inward,
    }
}

/// Applies every wall whose band contains node `(i, j)`.
pub(crate) fn apply_walls(
    mut velocity: DVec2,
    i: usize,
    j: usize,
    grid: &Grid,
    walls: Walls,
) -> DVec2 {
    let [nodes_x, nodes_y] = grid.nodes();
    if i <= WALL_CELLS {
        velocity = apply_wall(velocity, walls.left, DVec2::X);
    }
    if i + WALL_CELLS >= nodes_x - 1 {
        velocity = apply_wall(velocity, walls.right, DVec2::NEG_X);
    }
    if j <= WALL_CELLS {
        velocity = apply_wall(velocity, walls.bottom, DVec2::Y);
    }
    if j + WALL_CELLS >= nodes_y - 1 {
        velocity = apply_wall(velocity, walls.top, DVec2::NEG_Y);
    }
    velocity
}

/// Resolves contact of one body with the others sharing a grid node.
///
/// `velocity` is the body's node velocity, `centre` the mass-weighted mean
/// velocity of all bodies at the node and `outward` a vector along the
/// body's outward surface normal. If the body moves into the others, the
/// normal part of its velocity relative to `centre` is removed and Coulomb
/// friction with coefficient `friction` opposes the tangential part.
/// Separating bodies are left alone.
///
/// With two bodies and opposite normals the changes of their momenta cancel
/// exactly.
pub(crate) fn resolve_contact(
    velocity: DVec2,
    centre: DVec2,
    outward: DVec2,
    friction: f64,
) -> DVec2 {
    let Some(normal) = outward.try_normalize() else {
        return velocity;
    };
    let relative = velocity - centre;
    let approach = relative.dot(normal);
    if approach <= 0.0 {
        return velocity;
    }
    let tangential = relative - approach * normal;
    let slip = tangential.length();
    let friction_change = if slip > 0.0 {
        tangential * ((friction * approach).min(slip) / slip)
    } else {
        DVec2::ZERO
    };
    velocity - approach * normal - friction_change
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use rap_solver::Wall;

    use super::{apply_wall, resolve_contact};

    #[test]
    fn frictionless_wall_removes_only_inward_motion() {
        let into_wall = apply_wall(DVec2::new(-3.0, 2.0), Wall::Frictionless, DVec2::X);
        assert_eq!(into_wall, DVec2::new(0.0, 2.0));
        let away = apply_wall(DVec2::new(3.0, 2.0), Wall::Frictionless, DVec2::X);
        assert_eq!(away, DVec2::new(3.0, 2.0));
    }

    #[test]
    fn sliding_wall_removes_normal_motion_both_ways() {
        assert_eq!(
            apply_wall(DVec2::new(4.0, 3.0), Wall::Sliding, DVec2::Y),
            DVec2::new(4.0, 0.0)
        );
        assert_eq!(
            apply_wall(DVec2::new(4.0, -3.0), Wall::Sliding, DVec2::Y),
            DVec2::new(4.0, 0.0)
        );
    }

    #[test]
    fn sticky_wall_stops_everything() {
        assert_eq!(
            apply_wall(DVec2::new(3.0, 2.0), Wall::Sticky, DVec2::NEG_Y),
            DVec2::ZERO
        );
    }

    /// Two bodies meeting at a node, the first to the left of the second.
    fn collide(first: DVec2, second: DVec2, masses: (f64, f64), friction: f64) -> (DVec2, DVec2) {
        let centre = (masses.0 * first + masses.1 * second) / (masses.0 + masses.1);
        (
            resolve_contact(first, centre, DVec2::X, friction),
            resolve_contact(second, centre, DVec2::NEG_X, friction),
        )
    }

    #[test]
    fn approaching_bodies_share_their_normal_velocity_and_conserve_momentum() {
        let masses = (2.0, 3.0);
        let (first, second) = (DVec2::new(10.0, 1.0), DVec2::new(-5.0, -2.0));
        let (after_first, after_second) = collide(first, second, masses, 0.0);
        assert!((after_first.x - after_second.x).abs() < 1e-12);
        // Without friction the tangential velocities are untouched.
        assert!((after_first.y - 1.0).abs() < 1e-12 && (after_second.y + 2.0).abs() < 1e-12);
        let before = masses.0 * first + masses.1 * second;
        let after = masses.0 * after_first + masses.1 * after_second;
        assert!((after - before).length() < 1e-12);
    }

    #[test]
    fn separating_bodies_are_left_alone() {
        let (first, second) = (DVec2::new(-1.0, 0.5), DVec2::new(2.0, 0.0));
        assert_eq!(collide(first, second, (1.0, 1.0), 0.5), (first, second));
    }

    #[test]
    fn friction_is_limited_by_the_normal_impulse_and_never_reverses_slip() {
        let masses = (1.0, 1.0);
        let (first, second) = (DVec2::new(2.0, 10.0), DVec2::new(-2.0, -10.0));
        // Relative normal speed to the centre is 2, slip 10: friction 0.5
        // removes 1 from each body's tangential velocity.
        let (sliding, _) = collide(first, second, masses, 0.5);
        assert!((sliding - DVec2::new(0.0, 9.0)).length() < 1e-12);
        // Very high friction sticks the bodies instead of reversing them.
        let (stuck_first, stuck_second) = collide(first, second, masses, 100.0);
        assert!(stuck_first.length() < 1e-12 && stuck_second.length() < 1e-12);
    }

    #[test]
    fn missing_normal_disables_contact() {
        let velocity = DVec2::new(1.0, 2.0);
        assert_eq!(
            resolve_contact(velocity, DVec2::ZERO, DVec2::ZERO, 0.3),
            velocity
        );
    }
}
