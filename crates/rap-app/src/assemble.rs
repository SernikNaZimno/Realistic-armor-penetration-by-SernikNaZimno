//! Turns a scenario into a solver model.
//!
//! The projectile flies along `+x`. A perpendicular impact is modelled
//! axisymmetrically, with the shot line as the axis. An oblique impact has
//! no axis of symmetry; until a three-dimensional solver exists it is
//! modelled in plane strain, in which the projectile is a slab of the
//! profile's thickness rather than a body of revolution.
//!
//! All armor layers form one body, so adjacent layers are perfectly bonded.
//! A ballistic cap is a body of its own.

use glam::DVec2;
use rap_domain::{Armor, BallisticCap, MaterialId, Projectile, Scenario, SimulationSettings};
use rap_geometry::{fill_rectangle, nose};
use rap_materials::{Material, library};
use rap_solver::model::count_to_f64;
use rap_solver::{Grid, Model, ParticleSeed, SolverConfig, Symmetry, WALL_CELLS, Wall, Walls};

use crate::AppError;

/// Largest grid the assembler will create.
pub const MAX_NODES: usize = 8_000_000;

/// Obliquity below which an impact is treated as perpendicular, in rad.
const PERPENDICULAR_TOLERANCE: f64 = 1.0e-9;
/// Free cells between the wall bands and the nearest material.
const PAD_CELLS: f64 = 3.0;
/// Free cells between the projectile tip and the armor at the start.
const STANDOFF_CELLS: f64 = 1.5;
/// Free cells behind the armor for whatever emerges from it, at least.
const MIN_EXIT_CELLS: f64 = 10.0;
/// Free cells beside the armor for material thrown sideways.
const SIDE_PAD_CELLS: f64 = 8.0;
/// Initial temperature of all material, in K.
const INITIAL_TEMPERATURE: f64 = 294.0;

/// Where things are in an assembled model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// Geometric interpretation of the model.
    pub symmetry: Symmetry,
    /// Grid cell size, in m.
    pub cell_size: f64,
    /// `y` coordinate of the shot line, in m.
    pub shot_line: f64,
    /// `x` at which the shot line enters the armor, in m.
    pub armor_face: f64,
    /// `x` at which the shot line leaves the armor, in m.
    pub armor_back: f64,
    /// Time to simulate, in s.
    pub end_time: f64,
    /// Body index of the penetrator.
    pub projectile_body: usize,
    /// Body index of the ballistic cap, if the projectile has one.
    pub cap_body: Option<usize>,
    /// Body index of the armor.
    pub armor_body: usize,
}

/// A solver model together with the layout it was built on.
#[derive(Debug, Clone, PartialEq)]
pub struct Assembly {
    /// The model to hand to a solver.
    pub model: Model,
    /// Where the projectile and armor are in the model.
    pub layout: Layout,
}

/// Materials used by a model, in the order of their indices.
#[derive(Default)]
struct MaterialTable {
    materials: Vec<Material>,
}

impl MaterialTable {
    /// Index of the material `id`, adding it to the table on first use.
    /// Custom materials take precedence over built-in ones.
    fn index_of(&mut self, id: &MaterialId, custom: &[Material]) -> Result<usize, AppError> {
        if let Some(index) = self
            .materials
            .iter()
            .position(|material| &material.id == id)
        {
            return Ok(index);
        }
        let material = custom
            .iter()
            .cloned()
            .chain(library::builtin())
            .find(|material| &material.id == id)
            .ok_or_else(|| AppError::UnknownMaterial(id.clone()))?;
        self.materials.push(material);
        Ok(self.materials.len() - 1)
    }
}

/// Which part of a projectile a point belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Body,
    Cap,
}

/// Profile of a projectile as a body of revolution, measured from its
/// foremost point backwards.
struct Profile<'a> {
    projectile: &'a Projectile,
    nose_length: f64,
    /// Distance from the foremost point to the tip of the solid nose; zero
    /// unless a ballistic cap reaches further forward than the nose.
    cap_lead: f64,
}

impl<'a> Profile<'a> {
    fn new(projectile: &'a Projectile) -> Self {
        let nose_length = nose::length(projectile.nose(), projectile.body_radius());
        let cap_lead = projectile
            .cap()
            .map_or(0.0, |cap| (cap.length - nose_length).max(0.0));
        Self {
            projectile,
            nose_length,
            cap_lead,
        }
    }

    /// Overall length from the foremost point to the base.
    fn length(&self) -> f64 {
        self.cap_lead + self.nose_length + self.projectile.body_length()
    }

    /// Part of the projectile at `behind` the foremost point and `radial`
    /// from the axis, if any.
    fn part_at(&self, behind: f64, radial: f64) -> Option<Part> {
        let radius = self.projectile.body_radius();
        let behind_nose_tip = behind - self.cap_lead;
        let solid_length = self.nose_length + self.projectile.body_length();
        if (0.0..=solid_length).contains(&behind_nose_tip)
            && radial <= nose::radius(self.projectile.nose(), radius, behind_nose_tip)
        {
            return Some(Part::Body);
        }
        let cap = self.projectile.cap()?;
        // The cap starts `cap.length` ahead of the shoulder of the nose.
        let behind_cap_tip = behind_nose_tip - self.nose_length + cap.length;
        if !(0.0..=cap.length).contains(&behind_cap_tip) {
            return None;
        }
        let outer = nose::cap_radius(radius, cap.length, behind_cap_tip);
        (radial <= outer && radial >= outer - cap.wall_thickness).then_some(Part::Cap)
    }
}

/// Armor layers as intervals along the plate normal, measured from the face.
struct Stack {
    /// `(start, end, material index)` of every layer.
    layers: Vec<(f64, f64, usize)>,
    thickness: f64,
}

impl Stack {
    fn new(
        armor: &Armor,
        table: &mut MaterialTable,
        custom: &[Material],
    ) -> Result<Self, AppError> {
        let mut layers = Vec::with_capacity(armor.layers.len());
        let mut depth = 0.0;
        for (index, layer) in armor.layers.iter().enumerate() {
            // The space in front of the first plate is just standoff.
            if index > 0 {
                depth += layer.gap_before;
            }
            let material = table.index_of(&layer.material, custom)?;
            layers.push((depth, depth + layer.thickness, material));
            depth += layer.thickness;
        }
        Ok(Self {
            layers,
            thickness: depth,
        })
    }

    /// Material index at `depth` behind the face along the normal, if a
    /// plate is there.
    fn material_at(&self, depth: f64) -> Option<usize> {
        self.layers
            .iter()
            .find(|(start, end, _)| (*start..*end).contains(&depth))
            .map(|(_, _, material)| *material)
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is a small positive number of cells, rounded up"
)]
fn cells(length: f64, cell_size: f64) -> usize {
    (length / cell_size).ceil() as usize
}

/// Rounds `length` up to a whole number of `spacing`.
fn round_up(length: f64, spacing: f64) -> f64 {
    (length / spacing - 1.0e-9).ceil().max(0.0) * spacing
}

/// Builds the solver model of `scenario`.
///
/// `custom_materials` are searched before the built-in library.
///
/// # Errors
/// Returns [`AppError`] if the scenario or settings are invalid, a material
/// is unknown, or the grid would be unreasonably large.
#[expect(clippy::too_many_lines, reason = "one linear sequence of layout steps")]
pub fn assemble(
    scenario: &Scenario,
    settings: &SimulationSettings,
    custom_materials: &[Material],
) -> Result<Assembly, AppError> {
    scenario.validate()?;
    settings.validate().map_err(rap_domain::DomainError::from)?;

    let projectile = &scenario.projectile;
    let profile = Profile::new(projectile);
    let radius = projectile.body_radius();
    let obliquity = scenario.armor.obliquity;
    let symmetry = if obliquity.abs() < PERPENDICULAR_TOLERANCE {
        Symmetry::Axisymmetric
    } else {
        Symmetry::Planar
    };

    let cell_size = radius / settings.cells_per_radius;
    let spacing = cell_size / f64::from(settings.particles_per_cell_edge);
    let wall = count_to_f64(WALL_CELLS) * cell_size;
    let pad = PAD_CELLS * cell_size;

    let mut table = MaterialTable::default();
    let body_material = table.index_of(projectile.material(), custom_materials)?;
    let cap_material = projectile
        .cap()
        .map(|cap: &BallisticCap| table.index_of(&cap.material, custom_materials))
        .transpose()?;
    let stack = Stack::new(&scenario.armor, &mut table, custom_materials)?;

    // Plate normal and in-plane direction; the face passes through the
    // point where the shot line meets it.
    let normal = DVec2::new(obliquity.cos(), obliquity.sin());
    let along_plate = DVec2::new(-obliquity.sin(), obliquity.cos());
    let half_width = settings.armor_extent_radii * radius;
    let path_length = stack.thickness / obliquity.cos();

    // Positions along the shot line.
    let length = round_up(profile.length(), spacing);
    let standoff = STANDOFF_CELLS * cell_size + radius * obliquity.tan();
    let tilt_reach = half_width * obliquity.sin();
    let armor_face = (wall + pad + length + standoff).max(wall + pad + tilt_reach);
    let tip = armor_face - standoff;
    let exit_space = (0.5 * length).max(MIN_EXIT_CELLS * cell_size);
    let right =
        armor_face + path_length.max(stack.thickness * obliquity.cos() + tilt_reach) + exit_space;

    // Positions across the shot line.
    let side_pad = SIDE_PAD_CELLS * cell_size;
    let (shot_line, below, above) = match symmetry {
        Symmetry::Axisymmetric => (wall, 0.0, half_width),
        Symmetry::Planar => {
            let below = (half_width * obliquity.cos()).max(radius);
            let above = stack.thickness * obliquity.sin() + half_width * obliquity.cos();
            (
                wall + side_pad + round_up(below, spacing),
                below,
                above.max(radius),
            )
        }
    };
    let top = shot_line + above + side_pad;

    let grid = Grid {
        cells: [
            cells(right, cell_size) + WALL_CELLS,
            cells(top, cell_size) + WALL_CELLS,
        ],
        cell_size,
    };
    if grid.node_count() > MAX_NODES {
        return Err(AppError::GridTooLarge {
            nodes: grid.node_count(),
        });
    }

    let volume = |position: DVec2| match symmetry {
        Symmetry::Planar => spacing * spacing,
        // Per radian: cross-section area times radius.
        Symmetry::Axisymmetric => spacing * spacing * (position.y - shot_line),
    };

    let projectile_body = 0;
    let cap_body = cap_material.map(|_| 1);
    let armor_body = cap_body.map_or(1, |cap| cap + 1);

    let radial_extent = round_up(radius, spacing);
    let projectile_lower = match symmetry {
        Symmetry::Axisymmetric => shot_line,
        Symmetry::Planar => shot_line - radial_extent,
    };
    let velocity = DVec2::new(scenario.impact_speed, 0.0);
    let projectile_seeds = fill_rectangle(
        DVec2::new(tip - length, projectile_lower),
        DVec2::new(tip, shot_line + radial_extent),
        spacing,
    )
    .into_iter()
    .filter_map(|position| {
        let part = profile.part_at(tip - position.x, (position.y - shot_line).abs())?;
        let (material, body) = match (part, cap_material, cap_body) {
            (Part::Cap, Some(material), Some(body)) => (material, body),
            _ => (body_material, projectile_body),
        };
        Some(ParticleSeed {
            position,
            velocity,
            volume: volume(position),
            material,
            body,
        })
    });

    let face_point = DVec2::new(armor_face, shot_line);
    let armor_seeds = fill_rectangle(
        DVec2::new(
            armor_face - round_up(tilt_reach, spacing),
            shot_line - round_up(below, spacing),
        ),
        DVec2::new(right - exit_space, shot_line + above),
        spacing,
    )
    .into_iter()
    .filter_map(|position| {
        let offset = position - face_point;
        if offset.dot(along_plate).abs() > half_width {
            return None;
        }
        let material = stack.material_at(offset.dot(normal))?;
        Some(ParticleSeed {
            position,
            velocity: DVec2::ZERO,
            volume: volume(position),
            material,
            body: armor_body,
        })
    });

    let seeds = projectile_seeds.chain(armor_seeds).collect();
    // Nothing surrounds the impact: whatever reaches the edge of the grid
    // flies on and is no longer simulated. Only the axis is a real boundary.
    let open = Walls {
        left: Wall::Outflow,
        right: Wall::Outflow,
        bottom: Wall::Outflow,
        top: Wall::Outflow,
    };
    let walls = match symmetry {
        Symmetry::Axisymmetric => Walls {
            bottom: Wall::Sliding,
            ..open
        },
        Symmetry::Planar => open,
    };
    let model = Model {
        grid,
        symmetry,
        walls,
        config: SolverConfig {
            friction: settings.friction,
            ..SolverConfig::default()
        },
        materials: table.materials,
        seeds,
        initial_temperature: INITIAL_TEMPERATURE,
    };
    let end_time = settings
        .end_time
        .unwrap_or(2.0 * (length + path_length) / scenario.impact_speed);
    let layout = Layout {
        symmetry,
        cell_size,
        shot_line,
        armor_face,
        armor_back: armor_face + path_length,
        end_time,
        projectile_body,
        cap_body,
        armor_body,
    };
    Ok(Assembly { model, layout })
}
