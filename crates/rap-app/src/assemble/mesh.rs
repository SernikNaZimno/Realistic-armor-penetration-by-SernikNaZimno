//! Turns a scenario whose armor is a mesh into a solver model.
//!
//! The mesh is moved into the frame of the shot: `x` along the flight of
//! the projectile, the origin where the shot line enters the solid, and `y`
//! as close to the upward axis of the file as the shot line allows. The
//! part of the solid within the armor half-width of the shot line is filled
//! with particles. Nothing about the shape is symmetric, so the model is
//! the whole of it, not a half.

use std::path::Path;

use glam::{DVec2, DVec3};
use rap_domain::{DomainError, Scenario, SimulationSettings, SolidArmor};
use rap_geometry::{Columns, TriangleMesh, fill_box};
use rap_io::stl;
use rap_materials::Material;
use rap_solver::model::count_to_f64;
use rap_solver::{Grid3, Model3, ParticleSeed3, Symmetry, WALL_CELLS, Wall, Walls3};

use super::{
    INITIAL_TEMPERATURE, Layout, MAX_NODES, MIN_EXIT_CELLS, MaterialTable, ModelKind, PAD_CELLS,
    PROJECTILE_BODY, Parts, SIDE_PAD_CELLS, STANDOFF_CELLS, SolidAssembly, cells, round_up,
};
use crate::AppError;

/// Cosine of the angle to the upward axis of the file beyond which the shot
/// is treated as vertical and another axis is taken as "up".
const VERTICAL_LIMIT: f64 = 0.999;
/// Cells around the projectile within which armor counts as being in its
/// path when the projectile is placed.
const CLEARANCE_CELLS: f64 = 2.0;

/// Reads the mesh that the armor of `scenario` refers to, if it is given
/// by one. A relative file name starts at `folder`.
///
/// # Errors
/// Returns [`AppError::MeshFile`] if the file cannot be read or is not a
/// usable STL mesh.
pub fn load_mesh(scenario: &Scenario, folder: &Path) -> Result<Option<TriangleMesh>, AppError> {
    let Some(solid) = scenario.armor.solid() else {
        return Ok(None);
    };
    stl::load(&folder.join(&solid.file))
        .map(Some)
        .map_err(|error| AppError::MeshFile(error.to_string()))
}

/// The frame of the shot in the coordinates of the mesh file.
struct Frame {
    aim_point: DVec3,
    metres_per_unit: f64,
    /// Directions of `x`, `y` and `z` of the model, as unit vectors of the
    /// file.
    axes: [DVec3; 3],
}

impl Frame {
    fn new(solid: &SolidArmor) -> Self {
        let along = DVec3::from_array(solid.direction).normalize();
        let reference = if along.z.abs() < VERTICAL_LIMIT {
            DVec3::Z
        } else {
            DVec3::Y
        };
        let up = (reference - reference.dot(along) * along).normalize();
        Self {
            aim_point: DVec3::from_array(solid.aim_point),
            metres_per_unit: solid.metres_per_unit,
            axes: [along, up, along.cross(up)],
        }
    }

    /// Where the point `point` of the file is in the model, in m, relative
    /// to the aim point.
    fn place(&self, point: DVec3) -> DVec3 {
        let offset = (point - self.aim_point) * self.metres_per_unit;
        DVec3::from_array(self.axes.map(|axis| offset.dot(axis)))
    }
}

/// The armor as particles in the frame of the shot, whose origin is the
/// point where the shot line enters the solid.
struct PlacedArmor {
    points: Vec<DVec3>,
    /// `x` at which the shot line leaves the modelled part of the solid.
    back: f64,
    /// Smallest `x` of the armor in the path of the projectile.
    first_contact: f64,
    /// Corners of the box that just contains the particles' volumes.
    lower: DVec3,
    upper: DVec3,
}

impl PlacedArmor {
    fn new(
        solid: &SolidArmor,
        mesh: &TriangleMesh,
        half_width: f64,
        reach: f64,
        spacing: f64,
    ) -> Result<Self, AppError> {
        let frame = Frame::new(solid);
        let around_aim = mesh.mapped(|point| frame.place(point));
        let on_shot_line = around_aim
            .crossings(&Columns::single(DVec2::ZERO))?
            .swap_remove(0);
        let (Some(&entry), Some(&exit)) = (on_shot_line.first(), on_shot_line.last()) else {
            return Err(AppError::ShotMissesMesh);
        };
        let placed = around_aim.mapped(|point| point - entry * DVec3::X);

        let (mesh_lower, mesh_upper) = placed.bounds();
        let depth = solid
            .max_depth
            .map_or(mesh_upper.x, |limit| limit.min(mesh_upper.x));
        let half_width = round_up(half_width, spacing);
        let points = placed.fill(
            DVec3::new(-round_up(-mesh_lower.x, spacing), -half_width, -half_width),
            DVec3::new(round_up(depth, spacing), half_width, half_width),
            spacing,
        )?;
        if points.is_empty() {
            return Err(AppError::EmptyMesh);
        }
        let first_contact = points
            .iter()
            .filter(|point| point.y.hypot(point.z) <= reach)
            .map(|point| point.x)
            .fold(0.0, f64::min);
        let (lower, upper) = points.iter().fold(
            (DVec3::INFINITY, DVec3::NEG_INFINITY),
            |(lower, upper), point| (lower.min(*point), upper.max(*point)),
        );
        Ok(Self {
            points,
            back: (exit - entry).min(depth),
            first_contact: first_contact - 0.5 * spacing,
            lower: lower - 0.5 * spacing,
            upper: upper + 0.5 * spacing,
        })
    }
}

/// Builds the three-dimensional solver model of a `scenario` whose armor is
/// the solid enclosed by `mesh`.
///
/// `custom_materials` are searched before the built-in library.
///
/// # Errors
/// Returns [`AppError`] if the scenario or settings are invalid, the armor
/// is not given by a mesh, a material is unknown, the mesh is not closed or
/// is missed by the shot line, or the grid would be unreasonably large.
pub fn assemble_mesh(
    scenario: &Scenario,
    settings: &SimulationSettings,
    custom_materials: &[Material],
    mesh: &TriangleMesh,
) -> Result<SolidAssembly, AppError> {
    scenario.validate()?;
    settings.validate().map_err(DomainError::from)?;
    let Some(solid) = scenario.armor.solid() else {
        return Err(AppError::WrongArmorKind);
    };
    let projectile = &scenario.projectile;
    let radius = projectile.body_radius();
    let cell_size = radius / settings.cells_per_radius;
    let spacing = cell_size / f64::from(settings.particles_per_cell_edge);

    let mut table = MaterialTable::default();
    let parts = Parts::new(projectile, &mut table, custom_materials)?;
    let armor_material = table.index_of(&solid.material, custom_materials)?;
    let armor = PlacedArmor::new(
        solid,
        mesh,
        settings.armor_extent_radii * radius,
        radius + CLEARANCE_CELLS * cell_size,
        spacing,
    )?;

    // The projectile, in the frame of the shot.
    let length = round_up(parts.profile.length(), spacing);
    let radial_extent = round_up(radius, spacing);
    let tip = armor.first_contact - STANDOFF_CELLS * cell_size;
    let projectile_lower = DVec3::new(tip - length, -radial_extent, -radial_extent);
    let projectile_upper = DVec3::new(tip, radial_extent, radial_extent);

    // From the frame of the shot to the grid.
    let wall = count_to_f64(WALL_CELLS) * cell_size;
    let side_pad = SIDE_PAD_CELLS * cell_size;
    let exit_space = (0.5 * length).max(MIN_EXIT_CELLS * cell_size);
    let lower = armor.lower.min(projectile_lower);
    let upper = armor.upper.max(projectile_upper);
    let shift = DVec3::new(
        wall + PAD_CELLS * cell_size,
        wall + side_pad,
        wall + side_pad,
    ) - lower;
    let extent = upper + shift + DVec3::new(exit_space, side_pad, side_pad);
    let grid = Grid3 {
        cells: extent
            .to_array()
            .map(|extent| cells(extent, cell_size) + WALL_CELLS),
        cell_size,
    };
    if grid.node_count() > MAX_NODES {
        return Err(AppError::GridTooLarge {
            nodes: grid.node_count(),
        });
    }

    let volume = spacing.powi(3);
    let velocity = DVec3::new(scenario.impact_speed, 0.0, 0.0);
    let projectile_seeds = fill_box(projectile_lower, projectile_upper, spacing, |_| true)
        .into_iter()
        .filter_map(|point| {
            let (material, body) = parts.at(tip - point.x, point.y.hypot(point.z))?;
            Some(ParticleSeed3 {
                position: point + shift,
                velocity,
                volume,
                material,
                body,
            })
        });
    let armor_seeds = armor.points.iter().map(|point| ParticleSeed3 {
        position: *point + shift,
        velocity: DVec3::ZERO,
        volume,
        material: armor_material,
        body: parts.armor_body(),
    });
    let seeds = projectile_seeds.chain(armor_seeds).collect();

    let layout = Layout {
        kind: ModelKind::Solid,
        symmetry: Symmetry::Planar,
        cell_size,
        shot_line: shift.y,
        symmetry_plane: shift.z,
        half: false,
        armor_face: shift.x,
        armor_back: shift.x + armor.back,
        end_time: settings
            .end_time
            .unwrap_or(2.0 * (length + armor.back - tip) / scenario.impact_speed),
        projectile_body: PROJECTILE_BODY,
        cap_body: parts.cap_body(),
        armor_body: parts.armor_body(),
    };
    let model = Model3 {
        grid,
        // Nothing surrounds the impact on any side.
        walls: Walls3 {
            low: [Wall::Outflow; 3],
            high: [Wall::Outflow; 3],
        },
        config: super::config(settings.friction),
        materials: table.materials,
        seeds,
        initial_temperature: INITIAL_TEMPERATURE,
    };
    Ok(SolidAssembly { model, layout })
}
