//! Turns a scenario into a solver model.
//!
//! The projectile flies along `+x`. A perpendicular impact is modelled
//! axisymmetrically, with the shot line as the axis. An oblique impact has
//! no axis of symmetry; it is modelled either in three dimensions, as the
//! half on one side of the plane containing the shot line and the plate
//! normal, or, much cheaper and only indicative, in plane strain, where the
//! projectile is a slab of the profile's thickness.
//!
//! A face profile displaces the armor face, and every layer behind it,
//! along the plate normal as a function of the distance from the point
//! where the shot line meets the face.
//!
//! All armor layers form one body, so adjacent layers are perfectly bonded.
//! A ballistic cap is a body of its own; a bursting charge is part of the
//! body of its shell.
//!
//! Armor given by a mesh is assembled in [`mesh`].

pub mod mesh;

use glam::{DVec2, DVec3};
use rap_domain::{
    Armor, BallisticCap, DomainError, MaterialId, ObliqueModel, Projectile, Scenario,
    SimulationSettings, Target,
};
use rap_geometry::{fill_box, fill_rectangle, nose};
use rap_materials::{Material, library};
use rap_solver::model::count_to_f64;
use rap_solver::{
    Detonation, Grid, Grid3, Model, Model3, ParticleSeed, ParticleSeed3, SolverConfig, Symmetry,
    WALL_CELLS, Wall, Walls, Walls3,
};

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
/// Body index of the penetrator in every assembled model.
const PROJECTILE_BODY: usize = 0;
/// Initial temperature of all material, in K.
const INITIAL_TEMPERATURE: f64 = 294.0;

/// How a scenario is represented for the solver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// Two-dimensional, rotationally symmetric about the shot line.
    Axisymmetric,
    /// Two-dimensional plane strain.
    PlaneStrain,
    /// Three-dimensional: one half of a model with a symmetry plane for
    /// plates, the whole model for armor given by a mesh.
    Solid,
}

impl ModelKind {
    /// The kind of model `scenario` gets with `settings`.
    #[must_use]
    pub fn of(scenario: &Scenario, settings: &SimulationSettings) -> Self {
        match &scenario.armor {
            Target::Plates(armor) if armor.obliquity.abs() < PERPENDICULAR_TOLERANCE => {
                Self::Axisymmetric
            }
            Target::Plates(_) => match settings.oblique_model {
                ObliqueModel::Solid => Self::Solid,
                ObliqueModel::PlaneStrain => Self::PlaneStrain,
            },
            // A solid of any shape has no symmetry to exploit.
            _ => Self::Solid,
        }
    }
}

/// Where things are in an assembled model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// How the scenario is represented.
    pub kind: ModelKind,
    /// Symmetry of the two-dimensional picture of the model: the model
    /// itself, or for a solid model its cross-section in the symmetry plane.
    pub symmetry: Symmetry,
    /// Grid cell size, in m.
    pub cell_size: f64,
    /// `y` coordinate of the shot line, in m.
    pub shot_line: f64,
    /// `z` coordinate of the plane of a solid model that its
    /// two-dimensional picture shows: the symmetry plane of a half model,
    /// the plane through the shot line of a whole one. In m.
    pub symmetry_plane: f64,
    /// Whether a solid model is one half of a whole that is symmetric about
    /// [`Self::symmetry_plane`].
    pub half: bool,
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

/// A two-dimensional solver model together with its layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Assembly {
    /// The model to hand to a solver.
    pub model: Model,
    /// Where the projectile and armor are in the model.
    pub layout: Layout,
}

/// A three-dimensional solver model together with its layout.
#[derive(Debug, Clone, PartialEq)]
pub struct SolidAssembly {
    /// The model to hand to a solver.
    pub model: Model3,
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
    Filler,
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
            // The bursting charge fills a cylinder bored in from the base.
            let ahead_of_base = solid_length - behind_nose_tip;
            let in_cavity = self.projectile.filler().is_some_and(|filler| {
                radial <= filler.diameter / 2.0
                    && (filler.base_plug..=filler.base_plug + filler.length)
                        .contains(&ahead_of_base)
            });
            return Some(if in_cavity { Part::Filler } else { Part::Body });
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

/// The bodies and materials of a projectile.
struct Parts<'a> {
    profile: Profile<'a>,
    body_material: usize,
    cap_material: Option<usize>,
    filler_material: Option<usize>,
}

impl<'a> Parts<'a> {
    fn new(
        projectile: &'a Projectile,
        table: &mut MaterialTable,
        custom: &[Material],
    ) -> Result<Self, AppError> {
        Ok(Self {
            profile: Profile::new(projectile),
            body_material: table.index_of(projectile.material(), custom)?,
            cap_material: projectile
                .cap()
                .map(|cap: &BallisticCap| table.index_of(&cap.material, custom))
                .transpose()?,
            filler_material: projectile
                .filler()
                .map(|filler| table.index_of(&filler.material, custom))
                .transpose()?,
        })
    }

    fn cap_body(&self) -> Option<usize> {
        self.cap_material.map(|_| 1)
    }

    fn armor_body(&self) -> usize {
        self.cap_body().map_or(1, |cap| cap + 1)
    }

    /// Material and body of the projectile at `behind` its foremost point
    /// and `radial` from the shot line, if it is there.
    fn at(&self, behind: f64, radial: f64) -> Option<(usize, usize)> {
        match (
            self.profile.part_at(behind, radial)?,
            self.cap_material,
            self.cap_body(),
        ) {
            (Part::Cap, Some(material), Some(body)) => Some((material, body)),
            // The charge is bonded to the shell: same body, other material.
            (Part::Filler, ..) => Some((
                self.filler_material.unwrap_or(self.body_material),
                PROJECTILE_BODY,
            )),
            _ => Some((self.body_material, PROJECTILE_BODY)),
        }
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

/// Default numerical parameters with the given contact `friction` and
/// the detonation of the charge, if it has one.
fn config(friction: f64, detonation: Option<Detonation>) -> SolverConfig {
    SolverConfig {
        friction,
        detonation,
        ..SolverConfig::default()
    }
}

/// The detonation of the charge of `projectile`, if it has a fuze: the
/// fuze delay after the `flight_time` to the first contact. The products
/// of the explosive are added to `table`.
fn detonation(
    projectile: &Projectile,
    parts: &Parts,
    table: &mut MaterialTable,
    flight_time: f64,
) -> Result<Option<Detonation>, AppError> {
    let Some((filler, material)) = projectile.filler().zip(parts.filler_material) else {
        return Ok(None);
    };
    let Some(delay) = filler.fuze_delay else {
        return Ok(None);
    };
    let charge = &table.materials[material];
    let (Some(explosive), Some(products)) = (charge.explosive, charge.products()) else {
        return Err(AppError::InertCharge(filler.material.clone()));
    };
    table.materials.push(products);
    Ok(Some(Detonation {
        time: flight_time + delay,
        material,
        products: table.materials.len() - 1,
        energy: explosive.energy,
    }))
}

/// Everything about a scenario's layout that does not depend on the number
/// of dimensions of the model: sizes, positions along and across the shot
/// line, materials and bodies.
struct Plan<'a> {
    armor: &'a Armor,
    parts: Parts<'a>,
    stack: Stack,
    materials: Vec<Material>,
    kind: ModelKind,
    cell_size: f64,
    spacing: f64,
    friction: f64,
    detonation: Option<Detonation>,
    /// Plate normal and in-plane direction, in the `x`–`y` plane.
    normal: DVec2,
    along_plate: DVec2,
    /// Half-width of the armor from the shot line.
    half_width: f64,
    /// Rounded overall length of the projectile and the radius it is
    /// sampled within.
    length: f64,
    radial_extent: f64,
    /// `x` of the projectile's foremost point and of the armor face.
    tip: f64,
    armor_face: f64,
    /// How far the armor reaches from its face: back along `-x`, forward
    /// along `+x`, and below and above the shot line.
    reach_back: f64,
    reach_forward: f64,
    below: f64,
    above: f64,
    path_length: f64,
    exit_space: f64,
    /// Free space beside the model, between it and the grid walls.
    side_pad: f64,
    shot_line: f64,
    end_time: f64,
}

impl<'a> Plan<'a> {
    fn new(
        scenario: &'a Scenario,
        settings: &SimulationSettings,
        custom_materials: &[Material],
        kind: ModelKind,
    ) -> Result<Self, AppError> {
        scenario.validate()?;
        settings.validate().map_err(DomainError::from)?;

        let Target::Plates(armor) = &scenario.armor else {
            return Err(AppError::WrongArmorKind);
        };
        let projectile = &scenario.projectile;
        let radius = projectile.body_radius();
        let obliquity = armor.obliquity;
        let cell_size = radius / settings.cells_per_radius;
        let spacing = cell_size / f64::from(settings.particles_per_cell_edge);
        let wall = count_to_f64(WALL_CELLS) * cell_size;
        let margin = settings.margin_radii * radius;
        let pad = PAD_CELLS * cell_size + margin;
        let side_pad = SIDE_PAD_CELLS * cell_size + margin;
        // The gap the projectile crosses before it can touch the armor.
        let gap = STANDOFF_CELLS * cell_size + settings.standoff;

        let mut table = MaterialTable::default();
        let parts = Parts::new(projectile, &mut table, custom_materials)?;
        let stack = Stack::new(armor, &mut table, custom_materials)?;
        let flight_time = gap / scenario.impact_speed;
        let detonation = detonation(projectile, &parts, &mut table, flight_time)?;

        let half_width = settings.armor_extent_radii * radius;
        // How far a shaped face reaches towards the projectile and away.
        let (lowest_offset, highest_offset) = armor.face_offset_range();
        let (protrusion, recess) = (-lowest_offset, highest_offset);
        let path_length = stack.thickness / obliquity.cos();

        // Positions along the shot line.
        let length = round_up(parts.profile.length(), spacing);
        let standoff = gap + radius * obliquity.tan() + protrusion;
        let reach_back = half_width * obliquity.sin() + protrusion;
        let armor_face = (wall + pad + length + standoff).max(wall + pad + reach_back);
        let reach_forward =
            path_length.max(stack.thickness * obliquity.cos() + reach_back) + recess;
        let exit_space = (0.5 * length).max(MIN_EXIT_CELLS * cell_size) + margin;

        // Positions across the shot line.
        let (shot_line, below, above) = if kind == ModelKind::Axisymmetric {
            (wall, 0.0, half_width)
        } else {
            let shape_reach = protrusion + recess;
            let below = (half_width * obliquity.cos() + shape_reach).max(radius);
            let above =
                stack.thickness * obliquity.sin() + half_width * obliquity.cos() + shape_reach;
            (
                wall + side_pad + round_up(below, spacing),
                below,
                above.max(radius),
            )
        };

        Ok(Self {
            armor,
            parts,
            stack,
            materials: table.materials,
            kind,
            cell_size,
            spacing,
            friction: settings.friction,
            detonation,
            normal: DVec2::new(obliquity.cos(), obliquity.sin()),
            along_plate: DVec2::new(-obliquity.sin(), obliquity.cos()),
            half_width,
            length,
            radial_extent: round_up(radius, spacing),
            tip: armor_face - standoff,
            armor_face,
            reach_back,
            reach_forward,
            below,
            above,
            path_length,
            exit_space,
            side_pad,
            shot_line,
            end_time: settings.end_time.unwrap_or(
                (settings.standoff + 2.0 * (length + path_length)) / scenario.impact_speed,
            ),
        })
    }

    /// `x` of the right end of the grid interior and `y` of its top.
    fn extent(&self) -> DVec2 {
        DVec2::new(
            self.armor_face + self.reach_forward + self.exit_space,
            self.shot_line + self.above + self.side_pad,
        )
    }

    /// Number of grid cells needed to enclose `extent` plus a wall band.
    fn cells_for(&self, extent: f64) -> usize {
        cells(extent, self.cell_size) + WALL_CELLS
    }

    /// Material of the armor at `offset` from the point where the shot line
    /// meets the face, in the `x`–`y` plane, and `out_of_plane` from that
    /// plane, if a plate is there.
    fn armor_at(&self, offset: DVec2, out_of_plane: f64) -> Option<usize> {
        let in_plane = offset.dot(self.along_plate);
        if in_plane.abs() > self.half_width || out_of_plane.abs() > self.half_width {
            return None;
        }
        let lateral = in_plane.hypot(out_of_plane);
        let depth = offset.dot(self.normal) - self.armor.face_offset(lateral);
        self.stack.material_at(depth)
    }

    /// Lower and upper corners, in the `x`–`y` plane, of a box containing
    /// the armor.
    fn armor_box(&self) -> (DVec2, DVec2) {
        (
            DVec2::new(
                self.armor_face - round_up(self.reach_back, self.spacing),
                self.shot_line - round_up(self.below, self.spacing),
            ),
            DVec2::new(
                self.armor_face + self.reach_forward,
                self.shot_line + self.above,
            ),
        )
    }

    /// Lower and upper corners, in the `x`–`y` plane, of a box containing
    /// the projectile.
    fn projectile_box(&self) -> (DVec2, DVec2) {
        let lower_y = if self.kind == ModelKind::Axisymmetric {
            self.shot_line
        } else {
            self.shot_line - self.radial_extent
        };
        (
            DVec2::new(self.tip - self.length, lower_y),
            DVec2::new(self.tip, self.shot_line + self.radial_extent),
        )
    }

    fn config(&self) -> SolverConfig {
        config(self.friction, self.detonation)
    }

    fn layout(&self, symmetry_plane: f64) -> Layout {
        Layout {
            kind: self.kind,
            symmetry: if self.kind == ModelKind::Axisymmetric {
                Symmetry::Axisymmetric
            } else {
                Symmetry::Planar
            },
            cell_size: self.cell_size,
            shot_line: self.shot_line,
            symmetry_plane,
            half: self.kind == ModelKind::Solid,
            armor_face: self.armor_face,
            armor_back: self.armor_face + self.path_length,
            end_time: self.end_time,
            projectile_body: PROJECTILE_BODY,
            cap_body: self.parts.cap_body(),
            armor_body: self.parts.armor_body(),
        }
    }
}

/// Builds the two-dimensional solver model of `scenario`: axisymmetric for
/// a perpendicular impact, plane strain for an oblique one.
///
/// `custom_materials` are searched before the built-in library.
///
/// # Errors
/// Returns [`AppError`] if the scenario or settings are invalid, the armor
/// is given by a mesh, a material is unknown, or the grid would be
/// unreasonably large.
pub fn assemble(
    scenario: &Scenario,
    settings: &SimulationSettings,
    custom_materials: &[Material],
) -> Result<Assembly, AppError> {
    let kind = match ModelKind::of(scenario, settings) {
        ModelKind::Solid => ModelKind::PlaneStrain,
        kind => kind,
    };
    let plan = Plan::new(scenario, settings, custom_materials, kind)?;
    let extent = plan.extent();
    let grid = Grid {
        cells: [plan.cells_for(extent.x), plan.cells_for(extent.y)],
        cell_size: plan.cell_size,
    };
    if grid.node_count() > MAX_NODES {
        return Err(AppError::GridTooLarge {
            nodes: grid.node_count(),
        });
    }

    let (spacing, shot_line) = (plan.spacing, plan.shot_line);
    let volume = |position: DVec2| match kind {
        // Per radian: cross-section area times radius.
        ModelKind::Axisymmetric => spacing * spacing * (position.y - shot_line),
        _ => spacing * spacing,
    };
    let velocity = DVec2::new(scenario.impact_speed, 0.0);
    let (lower, upper) = plan.projectile_box();
    let projectile_seeds = fill_rectangle(lower, upper, spacing)
        .into_iter()
        .filter_map(|position| {
            let (material, body) = plan
                .parts
                .at(plan.tip - position.x, (position.y - shot_line).abs())?;
            Some(ParticleSeed {
                position,
                velocity,
                volume: volume(position),
                material,
                body,
            })
        });
    let face_point = DVec2::new(plan.armor_face, shot_line);
    let (lower, upper) = plan.armor_box();
    let armor_seeds = fill_rectangle(lower, upper, spacing)
        .into_iter()
        .filter_map(|position| {
            let material = plan.armor_at(position - face_point, 0.0)?;
            Some(ParticleSeed {
                position,
                velocity: DVec2::ZERO,
                volume: volume(position),
                material,
                body: plan.parts.armor_body(),
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
    let (symmetry, walls) = if kind == ModelKind::Axisymmetric {
        (
            Symmetry::Axisymmetric,
            Walls {
                bottom: Wall::Sliding,
                ..open
            },
        )
    } else {
        (Symmetry::Planar, open)
    };
    let layout = plan.layout(0.0);
    let model = Model {
        grid,
        symmetry,
        walls,
        config: plan.config(),
        materials: plan.materials,
        seeds,
        initial_temperature: INITIAL_TEMPERATURE,
    };
    Ok(Assembly { model, layout })
}

/// Builds the three-dimensional solver model of `scenario`.
///
/// The model is the half with `z` above the symmetry plane, which contains
/// the shot line and the plate normal. The projectile is a body of
/// revolution about the shot line; the armor extends the same half-width
/// out of the plane as within it.
///
/// # Errors
/// Returns [`AppError`] if the scenario or settings are invalid, the armor
/// is given by a mesh, a material is unknown, or the grid would be
/// unreasonably large.
pub fn assemble_solid(
    scenario: &Scenario,
    settings: &SimulationSettings,
    custom_materials: &[Material],
) -> Result<SolidAssembly, AppError> {
    let plan = Plan::new(scenario, settings, custom_materials, ModelKind::Solid)?;
    let extent = plan.extent();
    let symmetry_plane = count_to_f64(WALL_CELLS) * plan.cell_size;
    let depth = symmetry_plane + plan.half_width + plan.side_pad;
    let grid = Grid3 {
        cells: [
            plan.cells_for(extent.x),
            plan.cells_for(extent.y),
            plan.cells_for(depth),
        ],
        cell_size: plan.cell_size,
    };
    if grid.node_count() > MAX_NODES {
        return Err(AppError::GridTooLarge {
            nodes: grid.node_count(),
        });
    }

    let (spacing, shot_line) = (plan.spacing, plan.shot_line);
    let volume = spacing.powi(3);
    let velocity = DVec3::new(scenario.impact_speed, 0.0, 0.0);
    let slab = |corners: (DVec2, DVec2), thickness: f64| {
        (
            corners.0.extend(symmetry_plane),
            corners
                .1
                .extend(symmetry_plane + round_up(thickness, spacing)),
        )
    };

    let (lower, upper) = slab(plan.projectile_box(), plan.radial_extent);
    let projectile_seeds = fill_box(lower, upper, spacing, |_| true)
        .into_iter()
        .filter_map(|position| {
            let radial = (position.y - shot_line).hypot(position.z - symmetry_plane);
            let (material, body) = plan.parts.at(plan.tip - position.x, radial)?;
            Some(ParticleSeed3 {
                position,
                velocity,
                volume,
                material,
                body,
            })
        });
    let face_point = DVec2::new(plan.armor_face, shot_line);
    let (lower, upper) = slab(plan.armor_box(), plan.half_width);
    let armor_seeds = fill_box(lower, upper, spacing, |_| true)
        .into_iter()
        .filter_map(|position| {
            let material = plan.armor_at(
                position.truncate() - face_point,
                position.z - symmetry_plane,
            )?;
            Some(ParticleSeed3 {
                position,
                velocity: DVec3::ZERO,
                volume,
                material,
                body: plan.parts.armor_body(),
            })
        });
    let seeds = projectile_seeds.chain(armor_seeds).collect();

    let layout = plan.layout(symmetry_plane);
    let model = Model3 {
        grid,
        // Open on every side except the symmetry plane.
        walls: Walls3 {
            low: [Wall::Outflow, Wall::Outflow, Wall::Sliding],
            high: [Wall::Outflow; 3],
        },
        config: plan.config(),
        materials: plan.materials,
        seeds,
        initial_temperature: INITIAL_TEMPERATURE,
    };
    Ok(SolidAssembly { model, layout })
}
