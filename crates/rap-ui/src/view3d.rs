//! Three-dimensional picture of a solid model: the surface of the material
//! as shaded patches, seen through a camera that orbits the model.
//!
//! Particles are gathered into voxels a little larger than their spacing.
//! A voxel with an empty neighbour lies on the surface; it is drawn as one
//! patch whose normal points away from the occupied voxels around it.
//! Patches are painted from the farthest to the nearest, so nearer material
//! covers what is behind it.

use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Sense, Shape, Vec2};
use glam::{Mat3, Vec3};
use rap_solver::Particle3;

use crate::view::{self, Field};

/// Voxel edge in particle spacings: large enough that stretched material
/// still fills its voxels.
const VOXEL_SPACINGS: f32 = 1.5;
/// Largest number of voxels along one axis; coarser voxels are used for a
/// model that would need more.
const MAX_VOXELS: f32 = 192.0;
/// Patch edge in voxel edges; patches overlap a little so that no gap
/// shows between them.
const PATCH_VOXELS: f32 = 1.7;
/// Brightness of a patch facing away from the light.
const AMBIENT: f32 = 0.32;
/// Direction from the model to the light, in view space: above the viewer's
/// left shoulder.
const LIGHT: Vec3 = Vec3::new(-0.42, 0.55, 0.72);
/// Patches turned further away from the viewer than this are behind the
/// body they belong to and are not drawn.
const BACK_FACE: f32 = -0.2;
/// Radians of rotation per point of pointer movement.
const ORBIT_SPEED: f32 = 0.008;
/// Steepest view from above or below, in radians.
const PITCH_LIMIT: f32 = 1.5;
/// Limits of the zoom factor relative to fitting the whole model.
const ZOOM_RANGE: (f32, f32) = (0.2, 80.0);
/// Fraction of the panel the model fills at zoom 1.
const FIT_MARGIN: f32 = 0.9;
/// Steps along one axis to the voxels that shape the normal of a patch.
/// Two voxels each way keep the normals of a slanted face from following
/// its staircase.
const STEPS: [i8; 5] = [-2, -1, 0, 1, 2];
/// Greys of the armor, by material index.
const ARMOR_GREYS: [u8; 4] = [150, 110, 185, 85];
/// The six voxels that share a face with a voxel.
const FACES: [[i32; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

#[expect(
    clippy::cast_possible_truncation,
    reason = "the picture is drawn in single precision"
)]
fn single(value: f64) -> f32 {
    value as f32
}

/// What the view keeps of one particle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CloudPoint {
    pub(crate) position: Vec3,
    pub(crate) material: usize,
    /// Whether the particle belongs to the armor rather than the
    /// projectile.
    pub(crate) armor: bool,
    /// Failed fraction: 0 or 1 for a particle, in between for an average.
    pub(crate) failed: f32,
    pub(crate) damage: f32,
    pub(crate) plastic_strain: f32,
    /// In gigapascals.
    pub(crate) pressure: f32,
    pub(crate) temperature: f32,
    pub(crate) speed: f32,
}

impl CloudPoint {
    /// Value of `field` in the unit of its label.
    pub(crate) fn value(&self, field: Field) -> f32 {
        match field {
            Field::Material => 0.0,
            Field::Damage => self.damage,
            Field::PlasticStrain => self.plastic_strain,
            Field::Pressure => self.pressure,
            Field::Temperature => self.temperature,
            Field::Speed => self.speed,
        }
    }
}

/// The particles of a solid model at one instant, as the view needs them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cloud {
    pub(crate) points: Vec<CloudPoint>,
    /// Typical distance between neighbouring particles, in m.
    pub(crate) spacing: f32,
    /// `z` of the plane through the shot line that a section cuts along.
    pub(crate) plane: f32,
}

impl Cloud {
    /// The view's copy of `particles`, of which those of `armor_body` are
    /// the armor. `plane` is the `z` of the plane of the model's
    /// two-dimensional picture. A model that is one `half` of a
    /// symmetric whole lies behind that plane as the viewer sees it, so its
    /// mirror image, which is the other half, is kept instead.
    pub(crate) fn new(particles: &[Particle3], plane: f64, half: bool, armor_body: usize) -> Self {
        let plane = single(plane);
        let mut volume = 0.0;
        let points = particles
            .iter()
            .map(|particle| {
                volume += particle.volume;
                let mut position = particle.position.as_vec3();
                if half {
                    position.z = 2.0 * plane - position.z;
                }
                CloudPoint {
                    position,
                    material: particle.material,
                    armor: particle.body == armor_body,
                    failed: f32::from(u8::from(particle.is_failed())),
                    damage: single(particle.damage),
                    plastic_strain: single(particle.plastic_strain),
                    pressure: single(particle.pressure * 1.0e-9),
                    temperature: single(particle.temperature),
                    speed: single(particle.velocity.length()),
                }
            })
            .collect::<Vec<_>>();
        let count = points.len().max(1);
        #[expect(
            clippy::cast_precision_loss,
            reason = "particle counts are far below 2^53"
        )]
        let mean_volume = volume / count as f64;
        Self {
            points,
            spacing: single(mean_volume.cbrt()).max(f32::MIN_POSITIVE),
            plane,
        }
    }
}

/// One patch of the surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Patch {
    /// Mean state of the particles in the voxel; `position` is their
    /// centre.
    pub(crate) point: CloudPoint,
    /// Unit vector pointing out of the material, or zero for a lone voxel.
    pub(crate) normal: Vec3,
}

/// The surface of the material of a [`Cloud`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Surface {
    pub(crate) patches: Vec<Patch>,
    /// Edge length of a voxel, in m.
    pub(crate) voxel: f32,
    /// Corners of the box containing the patches, in m.
    pub(crate) bounds: (Vec3, Vec3),
}

/// Sums of the particles of one voxel.
#[derive(Clone, Copy)]
struct Tally {
    cell: [i32; 3],
    count: f32,
    sum: CloudPoint,
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "voxel indices are bounded by MAX_VOXELS plus padding"
)]
mod index {
    /// Whole number of voxels at or below `value`.
    pub(super) fn floor(value: f32) -> i32 {
        value.floor() as i32
    }

    /// Position in a dense array of the voxel at `cell`, or `None` outside
    /// `dims`.
    pub(super) fn dense(cell: [i32; 3], dims: [i32; 3]) -> Option<usize> {
        let inside = (0..3).all(|axis| cell[axis] >= 0 && cell[axis] < dims[axis]);
        inside.then(|| (cell[0] + dims[0] * (cell[1] + dims[1] * cell[2])) as usize)
    }

    pub(super) fn count(value: i32) -> usize {
        value.max(0) as usize
    }

    pub(super) fn slot(value: usize) -> u32 {
        value as u32
    }
}

impl Tally {
    fn empty(cell: [i32; 3], first: &CloudPoint) -> Self {
        Self {
            cell,
            count: 0.0,
            sum: CloudPoint {
                position: Vec3::ZERO,
                material: first.material,
                armor: first.armor,
                failed: 0.0,
                damage: 0.0,
                plastic_strain: 0.0,
                pressure: 0.0,
                temperature: 0.0,
                speed: 0.0,
            },
        }
    }

    fn add(&mut self, point: &CloudPoint) {
        self.count += 1.0;
        self.sum.position += point.position;
        self.sum.failed += point.failed;
        self.sum.damage += point.damage;
        self.sum.plastic_strain += point.plastic_strain;
        self.sum.pressure += point.pressure;
        self.sum.temperature += point.temperature;
        self.sum.speed += point.speed;
    }

    /// Mean state of the particles added; the material is that of the
    /// first.
    fn mean(&self) -> CloudPoint {
        let scale = 1.0 / self.count;
        CloudPoint {
            position: self.sum.position * scale,
            material: self.sum.material,
            armor: self.sum.armor,
            failed: self.sum.failed * scale,
            damage: self.sum.damage * scale,
            plastic_strain: self.sum.plastic_strain * scale,
            pressure: self.sum.pressure * scale,
            temperature: self.sum.temperature * scale,
            speed: self.sum.speed * scale,
        }
    }
}

impl Surface {
    /// Finds the surface of `cloud`. With `section`, only the material
    /// behind the plane through the shot line is kept, so that the cut face
    /// shows what is inside.
    pub(crate) fn extract(cloud: &Cloud, section: bool) -> Self {
        let limit = cloud.plane + 0.25 * cloud.spacing;
        let kept = || {
            cloud
                .points
                .iter()
                .filter(move |point| !section || point.position.z <= limit)
        };
        let (lower, upper) = kept().fold(
            (Vec3::INFINITY, Vec3::NEG_INFINITY),
            |(lower, upper), point| (lower.min(point.position), upper.max(point.position)),
        );
        if lower.cmpgt(upper).any() {
            return Self {
                patches: Vec::new(),
                voxel: cloud.spacing,
                bounds: (Vec3::ZERO, Vec3::ONE),
            };
        }
        let voxel =
            (VOXEL_SPACINGS * cloud.spacing).max((upper - lower).max_element() / MAX_VOXELS);
        // One empty voxel all around, so that every neighbour exists.
        let origin = lower - voxel;
        let dims = ((upper - origin) / voxel)
            .to_array()
            .map(|extent| index::floor(extent) + 2);
        let cell_of = |position: Vec3| ((position - origin) / voxel).to_array().map(index::floor);

        // Slot of each voxel in `tallies`, plus one; zero for an empty voxel.
        let mut slots = vec![0_u32; dims.iter().map(|dim| index::count(*dim)).product()];
        let mut tallies: Vec<Tally> = Vec::new();
        for point in kept() {
            let cell = cell_of(point.position);
            let Some(dense) = index::dense(cell, dims) else {
                continue;
            };
            if slots[dense] == 0 {
                tallies.push(Tally::empty(cell, point));
                slots[dense] = index::slot(tallies.len());
            }
            let tally = &mut tallies[slots[dense] as usize - 1];
            tally.add(point);
        }

        let occupied =
            |cell: [i32; 3]| index::dense(cell, dims).is_some_and(|dense| slots[dense] != 0);
        let shifted =
            |cell: [i32; 3], by: [i32; 3]| [cell[0] + by[0], cell[1] + by[1], cell[2] + by[2]];
        let patches = tallies
            .iter()
            .filter(|tally| {
                FACES
                    .iter()
                    .any(|face| !occupied(shifted(tally.cell, *face)))
            })
            .map(|tally| {
                // Away from the occupied voxels around.
                let mut inward = Vec3::ZERO;
                for dz in STEPS {
                    for dy in STEPS {
                        for dx in STEPS {
                            if occupied(shifted(tally.cell, [dx, dy, dz].map(i32::from))) {
                                inward += Vec3::new(dx.into(), dy.into(), dz.into());
                            }
                        }
                    }
                }
                Patch {
                    point: tally.mean(),
                    normal: (-inward).normalize_or_zero(),
                }
            })
            .collect();
        Self {
            patches,
            voxel,
            bounds: (lower, upper),
        }
    }

    /// Range of `field` over the surface, never empty.
    pub(crate) fn range(&self, field: Field) -> (f64, f64) {
        if field == Field::Damage {
            return (0.0, 1.0);
        }
        let (lowest, highest) = self
            .patches
            .iter()
            .map(|patch| f64::from(patch.point.value(field)))
            .fold((f64::MAX, f64::MIN), |(lowest, highest), value| {
                (lowest.min(value), highest.max(value))
            });
        if lowest > highest {
            (0.0, 1.0)
        } else if highest - lowest < 1.0e-9 {
            (lowest, lowest + 1.0)
        } else {
            (lowest, highest)
        }
    }
}

/// Where the viewer stands relative to the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Camera3 {
    /// Rotation about the vertical axis, in radians.
    pub(crate) yaw: f32,
    /// Elevation above the horizontal, in radians.
    pub(crate) pitch: f32,
    /// Shift of the model on screen, in points.
    pub(crate) pan: Vec2,
    /// Magnification relative to fitting the model in the panel.
    pub(crate) zoom: f32,
}

impl Default for Camera3 {
    /// A little to the side of and above the section plane.
    fn default() -> Self {
        Self {
            yaw: -0.45,
            pitch: 0.3,
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera3 {
    /// Rotation from model axes to view axes: `x` to the right, `y` up and
    /// `z` towards the viewer.
    fn rotation(&self) -> Mat3 {
        Mat3::from_rotation_x(self.pitch) * Mat3::from_rotation_y(self.yaw)
    }
}

/// Colour of the material of a patch: grey for armor, so that a projectile
/// of the same material stands out against it; failed material is darker.
fn material_color(point: &CloudPoint) -> Color32 {
    let failed = point.failed > 0.5;
    if point.armor {
        let grey = ARMOR_GREYS[point.material % ARMOR_GREYS.len()];
        shaded(Color32::from_gray(grey), if failed { 0.6 } else { 1.0 })
    } else {
        view::material_color(point.material, failed)
    }
}

fn shaded(color: Color32, brightness: f32) -> Color32 {
    let scale = |channel: u8| {
        let value = (f32::from(channel) * brightness).round().clamp(0.0, 255.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the value is clamped to the range of a colour channel"
        )]
        let value = value as u8;
        value
    };
    Color32::from_rgb(scale(color.r()), scale(color.g()), scale(color.b()))
}

/// Builds one mesh with a shaded square per patch that faces the viewer,
/// farthest first.
pub(crate) fn surface_mesh(surface: &Surface, field: Field, camera: &Camera3, rect: Rect) -> Mesh {
    let rotation = camera.rotation();
    let centre = (surface.bounds.0 + surface.bounds.1) / 2.0;
    let reach = (surface.bounds.1 - surface.bounds.0).length().max(1.0e-9);
    let scale = rect.width().min(rect.height()) / reach * FIT_MARGIN * camera.zoom;
    let origin = rect.center() + camera.pan;
    let half = (PATCH_VOXELS * surface.voxel * scale / 2.0).max(0.5);
    let visible = rect.expand(half);
    let range = surface.range(field);
    let light = LIGHT.normalize();

    let mut drawn: Vec<(f32, Pos2, Color32)> = surface
        .patches
        .iter()
        .filter_map(|patch| {
            let normal = rotation * patch.normal;
            if normal.z < BACK_FACE {
                return None;
            }
            let seen = rotation * (patch.point.position - centre);
            let position = origin + Vec2::new(seen.x, -seen.y) * scale;
            if !visible.contains(position) {
                return None;
            }
            let base = match field {
                Field::Material => material_color(&patch.point),
                _ => view::gradient(
                    (f64::from(patch.point.value(field)) - range.0) / (range.1 - range.0),
                ),
            };
            // A lone voxel has no normal; light it evenly.
            let lit = if patch.normal == Vec3::ZERO {
                0.8
            } else {
                AMBIENT + (1.0 - AMBIENT) * normal.dot(light).max(0.0)
            };
            Some((seen.z, position, shaded(base, lit)))
        })
        .collect();
    drawn.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut mesh = Mesh::default();
    for (_, position, color) in drawn {
        mesh.add_colored_rect(
            Rect::from_center_size(position, Vec2::splat(2.0 * half)),
            color,
        );
    }
    mesh
}

/// Draws `surface` in the remaining space of `ui`. Dragging orbits the
/// camera, dragging with the secondary button pans, scrolling zooms and a
/// double click resets the camera.
pub(crate) fn show(ui: &mut egui::Ui, surface: &Surface, field: Field, camera: &mut Camera3) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = response.rect;
    painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);

    if response.dragged_by(egui::PointerButton::Primary) {
        let delta = response.drag_delta();
        camera.yaw += delta.x * ORBIT_SPEED;
        camera.pitch = (camera.pitch + delta.y * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    } else if response.dragged() {
        camera.pan += response.drag_delta();
    }
    if response.double_clicked() {
        *camera = Camera3::default();
    }
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            camera.zoom = (camera.zoom * (scroll * 0.003).exp()).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        }
    }

    painter.add(Shape::mesh(surface_mesh(surface, field, camera, rect)));
    view::legend(
        &painter,
        rect,
        field,
        surface.range(field),
        ui.visuals().text_color(),
    );
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Pos2, Rect};
    use glam::{DMat3, DVec3, Vec3};
    use rap_solver::Particle3;

    use super::{Camera3, Cloud, Surface, surface_mesh};
    use crate::view::Field;

    const SPACING: f64 = 1.0e-3;

    fn particle(position: DVec3, material: usize, damage: f64) -> Particle3 {
        Particle3 {
            position,
            velocity: DVec3::new(300.0, 0.0, 400.0),
            velocity_gradient: DMat3::ZERO,
            mass: 7.83e-6,
            volume: SPACING.powi(3),
            body: 0,
            material,
            deviator: DMat3::ZERO,
            pressure: 2.0e9,
            viscous_pressure: 0.0,
            specific_energy: 0.0,
            plastic_strain: 0.1,
            plastic_strain_rate: 0.0,
            temperature: 400.0,
            damage,
            sound_speed: 5000.0,
        }
    }

    /// A block of `count` particles along each axis, starting at `corner`.
    fn block(corner: DVec3, count: [u32; 3], material: usize) -> Vec<Particle3> {
        let mut particles = Vec::new();
        for k in 0..count[2] {
            for j in 0..count[1] {
                for i in 0..count[0] {
                    let offset = DVec3::new(f64::from(i), f64::from(j), f64::from(k)) + 0.5;
                    particles.push(particle(corner + offset * SPACING, material, 0.0));
                }
            }
        }
        particles
    }

    fn panel() -> Rect {
        Rect::from_min_max(Pos2::ZERO, Pos2::new(400.0, 300.0))
    }

    #[test]
    fn cloud_keeps_state_in_display_units_and_mirrors_a_half_model() {
        let particles = [particle(DVec3::new(0.01, 0.02, 0.013), 2, 1.0)];
        let whole = Cloud::new(&particles, 0.01, false, 0);
        let point = whole.points[0];
        assert!((point.position - Vec3::new(0.01, 0.02, 0.013)).length() < 1e-7);
        assert_eq!(point.material, 2);
        assert!((point.failed - 1.0).abs() < f32::EPSILON);
        assert!((point.value(Field::Pressure) - 2.0).abs() < 1e-6);
        assert!((point.value(Field::Speed) - 500.0).abs() < 1e-3);
        assert!((point.value(Field::Temperature) - 400.0).abs() < 1e-3);
        assert!((whole.spacing - 1.0e-3).abs() < 1e-8);
        // 3 mm behind the plane becomes 3 mm in front of it.
        let half = Cloud::new(&particles, 0.01, true, 0);
        assert!((half.points[0].position.z - 0.007).abs() < 1e-7);
    }

    #[test]
    fn surface_of_a_block_is_its_skin_with_outward_normals() {
        let particles = block(DVec3::ZERO, [30, 30, 30], 0);
        let cloud = Cloud::new(&particles, 1.0, false, 0);
        let surface = Surface::extract(&cloud, false);
        // Voxels of 1.5 mm: 20 along each edge, of which the skin is drawn.
        assert!((surface.voxel - 1.5e-3).abs() < 1e-8);
        assert_eq!(surface.patches.len(), 20 * 20 * 20 - 18 * 18 * 18);
        for patch in &surface.patches {
            let from_centre = patch.point.position - Vec3::splat(0.015);
            assert!(patch.normal.dot(from_centre) > 0.0, "{patch:?}");
            assert!((patch.normal.length() - 1.0).abs() < 1e-5);
        }
        // In the middle of a face the normal is the face normal.
        let top = surface
            .patches
            .iter()
            .find(|patch| {
                let position = patch.point.position;
                position.y > 0.029
                    && (position.x - 0.015).abs() < 1e-3
                    && (position.z - 0.015).abs() < 1e-3
            })
            .expect("a patch in the middle of the top face");
        assert!((top.normal - Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn section_cuts_at_the_plane_and_shows_what_is_inside() {
        // A core of another material inside a block, cut through its middle.
        let mut particles = block(DVec3::ZERO, [12, 12, 12], 0);
        for particle in &mut particles {
            let from_axis = (particle.position.truncate() - 0.006).abs().max_element();
            if from_axis < 0.002 {
                particle.material = 1;
            }
        }
        let cloud = Cloud::new(&particles, 0.006, false, 0);
        let whole = Surface::extract(&cloud, false);
        assert!(whole.patches.iter().all(|patch| patch.point.material == 0
            || patch.point.position.z < 0.002
            || patch.point.position.z > 0.010));
        let cut = Surface::extract(&cloud, true);
        assert!(cut.bounds.1.z < 0.0065);
        let core_on_the_cut = cut
            .patches
            .iter()
            .filter(|patch| patch.point.material == 1 && patch.point.position.z > 0.004)
            .count();
        assert!(core_on_the_cut > 0);
        for patch in cut.patches.iter().filter(|patch| {
            patch.point.position.z > 0.004
                && (patch.point.position.truncate() - 0.006)
                    .abs()
                    .max_element()
                    < 0.003
        }) {
            assert!((patch.normal - Vec3::Z).length() < 1e-5);
        }
    }

    #[test]
    fn lone_fragments_and_empty_clouds_are_handled() {
        let lone = Cloud::new(&[particle(DVec3::splat(0.01), 0, 1.0)], 1.0, false, 0);
        let surface = Surface::extract(&lone, false);
        assert_eq!(surface.patches.len(), 1);
        assert_eq!(surface.patches[0].normal, Vec3::ZERO);
        assert_eq!(surface.range(Field::Damage), (0.0, 1.0));
        let mesh = surface_mesh(&surface, Field::Material, &Camera3::default(), panel());
        assert_eq!(mesh.vertices.len(), 4);

        let empty = Surface::extract(&Cloud::new(&[], 0.0, false, 0), true);
        assert_eq!(empty.patches, Vec::new());
        assert_eq!(empty.range(Field::Speed), (0.0, 1.0));
        let mesh = surface_mesh(&empty, Field::Speed, &Camera3::default(), panel());
        assert!(mesh.is_empty());
    }

    #[test]
    fn only_patches_facing_the_viewer_are_drawn_and_the_lit_side_is_brighter() {
        let cloud = Cloud::new(&block(DVec3::ZERO, [30, 30, 30], 0), 1.0, false, 1);
        let surface = Surface::extract(&cloud, false);
        let front = Camera3 {
            yaw: 0.0,
            pitch: 0.0,
            ..Camera3::default()
        };
        let mesh = surface_mesh(&surface, Field::Material, &front, panel());
        let quads = mesh.vertices.len() / 4;
        // Head on, the near face is drawn and the far face is not; the
        // faces seen edge-on are covered by the near one.
        assert!(
            quads >= 20 * 20 && quads <= surface.patches.len() - 20 * 20,
            "{quads}"
        );
    }

    #[test]
    fn patches_turned_towards_the_light_are_brighter() {
        // Two patches side by side, one facing left and one facing right;
        // the light comes from the viewer's left.
        let cloud = Cloud::new(&block(DVec3::ZERO, [2, 2, 2], 0), 1.0, false, 1);
        let mut surface = Surface::extract(&cloud, false);
        let mut patch = surface.patches[0];
        surface.patches.clear();
        for (x, normal) in [
            (0.0, Vec3::new(-1.0, 0.0, 0.3)),
            (0.01, Vec3::new(1.0, 0.0, 0.3)),
        ] {
            patch.point.position = Vec3::new(x, 0.0, 0.0);
            patch.normal = normal.normalize();
            surface.patches.push(patch);
        }
        surface.bounds = (Vec3::ZERO, Vec3::new(0.01, 0.0, 0.0));
        let head_on = Camera3 {
            yaw: 0.0,
            pitch: 0.0,
            ..Camera3::default()
        };
        let mesh = surface_mesh(&surface, Field::Material, &head_on, panel());
        assert_eq!(mesh.vertices.len(), 8);
        let (first, second) = (mesh.vertices[0], mesh.vertices[4]);
        let (left, right) = if first.pos.x < second.pos.x {
            (first, second)
        } else {
            (second, first)
        };
        assert!(left.color.r() > right.color.r() + 40);
    }

    /// Writes pictures of a scenario for a look at what the view draws:
    /// `RAP_PICTURES=<folder> RAP_SCENARIO=<file> RAP_TIMES=<µs,µs,...>
    /// cargo test -p rap-ui pictures -- --ignored`. The scenario defaults
    /// to the shell example and the times to three instants.
    #[test]
    #[ignore = "writes files; a development aid, not a check"]
    fn pictures_of_a_scenario() {
        use std::fmt::Write as _;

        use rap_app::{Simulation, presets};

        let Ok(folder) = std::env::var("RAP_PICTURES") else {
            return;
        };
        let mut file = match std::env::var("RAP_SCENARIO") {
            Ok(path) => rap_io::ScenarioFile::load(std::path::Path::new(&path))
                .expect("the scenario file is readable"),
            Err(_) => presets::aphe_against_cast_armor(),
        };
        let times: Vec<f64> = std::env::var("RAP_TIMES")
            .unwrap_or_else(|_| "0,300,900".to_owned())
            .split(',')
            .map(|time| time.trim().parse::<f64>().expect("times are numbers") * 1.0e-6)
            .collect();
        file.settings.backend = rap_domain::Backend::Gpu;
        let mut simulation = Simulation::new(&file.scenario, &file.settings, &file.materials)
            .expect("the scenario runs on this machine");
        eprintln!("{} particles", simulation.particle_count());
        let rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(900.0, 600.0));
        for time in times {
            let name = format!("{:04.0}", time * 1.0e6);
            simulation.advance_to(time).expect("the run stays valid");
            eprintln!("{name} µs: {:?}", simulation.summary());
            let layout = simulation.layout();
            let particles = simulation.solid_particles().expect("a solid model");
            let cloud = Cloud::new(
                &particles,
                layout.symmetry_plane,
                layout.half,
                layout.armor_body,
            );
            let surface = Surface::extract(&cloud, true);
            let angle = |name: &str, default: f32| {
                std::env::var(name)
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(default)
            };
            let camera = Camera3 {
                yaw: angle("RAP_YAW", Camera3::default().yaw),
                pitch: angle("RAP_PITCH", Camera3::default().pitch),
                ..Camera3::default()
            };
            for (field, label) in [
                (Field::Material, "material"),
                (Field::PlasticStrain, "strain"),
            ] {
                let mesh = surface_mesh(&surface, field, &camera, rect);
                let mut svg = String::from(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 900 600\"                      style=\"background:#1b1b1b\">
",
                );
                for quad in mesh.vertices.chunks_exact(4) {
                    let (from, to, color) = (quad[0].pos, quad[3].pos, quad[0].color);
                    let _ = writeln!(
                        svg,
                        "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\"                          fill=\"rgb({},{},{})\"/>",
                        from.x,
                        from.y,
                        to.x - from.x,
                        to.y - from.y,
                        color.r(),
                        color.g(),
                        color.b()
                    );
                }
                svg.push_str(
                    "</svg>
",
                );
                let path = std::path::Path::new(&folder).join(format!("{name}-{label}.svg"));
                std::fs::write(path, svg).expect("the picture is written");
            }
        }
    }
}
