//! The surface of a cloud of particles, and the camera it is seen through.
//!
//! Particles are gathered into voxels a little larger than their spacing.
//! A voxel with an empty neighbour lies on the surface; it becomes one
//! patch whose normal points away from the occupied voxels around it.

use std::f64::consts::PI;

use glam::{Mat3, Vec3};
use rap_solver::{Particle, Particle3};

use crate::palette::Field;

/// Voxel edge in particle spacings: large enough that stretched material
/// still fills its voxels.
const VOXEL_SPACINGS: f32 = 1.5;
/// Largest number of voxels along one axis; coarser voxels are used for a
/// model that would need more.
const MAX_VOXELS: f32 = 192.0;
/// Distance between the copies of a particle swept round an axis, in
/// particle spacings: close enough that every voxel on the way gets one.
const ARC_SPACINGS: f64 = 1.2;
/// Steps along one axis to the voxels that shape the normal of a patch.
/// Two voxels each way keep the normals of a slanted face from following
/// its staircase.
const STEPS: [i8; 5] = [-2, -1, 0, 1, 2];
/// Steps along one axis to the voxels whose normals are averaged.
const NEIGHBOURS: [i8; 3] = [-1, 0, 1];
/// Times the normals are averaged with those of their neighbours.
const SMOOTHING_PASSES: usize = 2;
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
pub struct CloudPoint {
    /// Position, in m.
    pub position: Vec3,
    /// Index of the material.
    pub material: usize,
    /// Whether the particle belongs to the armor rather than the
    /// projectile.
    pub armor: bool,
    /// Failed fraction: 0 or 1 for a particle, in between for an average.
    pub failed: f32,
    /// Accumulated damage, from 0 to 1.
    pub damage: f32,
    /// Equivalent plastic strain.
    pub plastic_strain: f32,
    /// In gigapascals.
    pub pressure: f32,
    /// Temperature, in K.
    pub temperature: f32,
    /// Speed, in m/s.
    pub speed: f32,
}

impl CloudPoint {
    /// Value of `field` in the unit of its label.
    #[must_use]
    pub fn value(&self, field: Field) -> f32 {
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
pub struct Cloud {
    /// The particles.
    pub points: Vec<CloudPoint>,
    /// Typical distance between neighbouring particles, in m.
    pub spacing: f32,
    /// `z` of the plane through the shot line that a section cuts along.
    pub plane: f32,
}

impl Cloud {
    /// The view's copy of `particles`, of which those of `armor_body` are
    /// the armor. `plane` is the `z` of the plane of the model's
    /// two-dimensional picture. A model that is one `half` of a
    /// symmetric whole lies behind that plane as the viewer sees it, so its
    /// mirror image, which is the other half, is kept instead.
    #[must_use]
    pub fn new(particles: &[Particle3], plane: f64, half: bool, armor_body: usize) -> Self {
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

    /// The far half of an axisymmetric model: every particle of its
    /// two-dimensional picture swept half-way round the axis at `y = axis`,
    /// so that a section along the plane `z = 0` shows that picture.
    /// Particles of `armor_body` are the armor.
    ///
    /// At most about `budget` points are made; if the model would need
    /// more, they are placed further apart and the cloud is that much
    /// coarser.
    #[must_use]
    pub fn revolved(particles: &[Particle], axis: f64, armor_body: usize, budget: usize) -> Self {
        // A particle of an axisymmetric model stands for a ring. Its volume
        // is per radian, so its cross-section is the volume over the radius.
        let radius_of = |particle: &Particle| (particle.position.y - axis).max(0.0);
        let (mut area, mut counted, mut sweep) = (0.0, 0.0, 0.0);
        for particle in particles {
            let radius = radius_of(particle);
            if radius > 0.0 {
                area += particle.volume / radius;
                counted += 1.0;
                sweep += PI * radius;
            }
        }
        let spacing = if counted > 0.0 {
            (area / counted).sqrt()
        } else {
            f64::MIN_POSITIVE
        };
        #[expect(
            clippy::cast_precision_loss,
            reason = "point counts are far below 2^53"
        )]
        let step = (ARC_SPACINGS * spacing).max(sweep / budget.max(1) as f64);

        let mut points = Vec::new();
        for particle in particles {
            let radius = radius_of(particle);
            let state = CloudPoint {
                position: Vec3::ZERO,
                material: particle.material,
                armor: particle.body == armor_body,
                failed: f32::from(u8::from(particle.is_failed())),
                damage: single(particle.damage),
                plastic_strain: single(particle.plastic_strain),
                pressure: single(particle.pressure * 1.0e-9),
                temperature: single(particle.temperature),
                speed: single(particle.velocity.length()),
            };
            let copies = arc_count(PI * radius / step);
            for copy in 0..copies {
                // Evenly over the half-turn behind the plane.
                let angle = -PI * (f64::from(copy) + 0.5) / f64::from(copies);
                let (sine, cosine) = angle.sin_cos();
                points.push(CloudPoint {
                    position: Vec3::new(
                        single(particle.position.x),
                        single(axis + radius * cosine),
                        single(radius * sine),
                    ),
                    ..state
                });
            }
        }
        Self {
            points,
            spacing: single(step / ARC_SPACINGS).max(f32::MIN_POSITIVE),
            plane: 0.0,
        }
    }
}

/// Number of copies of a particle along an arc that is `steps` steps long.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is a small positive number, rounded up"
)]
fn arc_count(steps: f64) -> u32 {
    steps.ceil().clamp(1.0, f64::from(u32::MAX)) as u32
}

/// One patch of the surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Patch {
    /// Mean state of the particles in the voxel; `position` is their
    /// centre.
    pub point: CloudPoint,
    /// Unit vector pointing out of the material, or zero for a lone voxel.
    pub normal: Vec3,
}

/// The surface of the material of a [`Cloud`].
#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    /// The patches, in no particular order.
    pub patches: Vec<Patch>,
    /// Edge length of a voxel, in m.
    pub voxel: f32,
    /// Corners of the box containing the patches, in m.
    pub bounds: (Vec3, Vec3),
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

/// Outward unit normal of every voxel of `tallies` that lies on the
/// surface; `None` for a voxel inside the material and zero for a lone
/// voxel. `slots` gives, for each voxel of a grid of `dims`, its index in
/// `tallies` plus one, or zero if it is empty.
fn normals_of(tallies: &[Tally], slots: &[u32], dims: [i32; 3]) -> Vec<Option<Vec3>> {
    let occupied = |cell: [i32; 3]| index::dense(cell, dims).is_some_and(|dense| slots[dense] != 0);
    let shifted =
        |cell: [i32; 3], by: [i32; 3]| [cell[0] + by[0], cell[1] + by[1], cell[2] + by[2]];
    // Raw normal of every voxel on the surface: away from the occupied
    // voxels around it. Voxels inside the material get none.
    let on_surface = |tally: &Tally| {
        FACES
            .iter()
            .any(|face| !occupied(shifted(tally.cell, *face)))
    };
    let mut normals: Vec<Option<Vec3>> = tallies
        .iter()
        .map(|tally| {
            on_surface(tally).then(|| {
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
                (-inward).normalize_or_zero()
            })
        })
        .collect();
    // A slanted face is a staircase of voxels whose raw normals take
    // turns; averaging each with its neighbours on the surface gives
    // the normal of the face.
    for _ in 0..SMOOTHING_PASSES {
        normals = tallies
            .iter()
            .zip(&normals)
            .map(|(tally, normal)| {
                let own = (*normal)?;
                let mut sum = Vec3::ZERO;
                for dz in NEIGHBOURS {
                    for dy in NEIGHBOURS {
                        for dx in NEIGHBOURS {
                            let cell = shifted(tally.cell, [dx, dy, dz].map(i32::from));
                            let near = index::dense(cell, dims)
                                .filter(|dense| slots[*dense] != 0)
                                .and_then(|dense| normals[slots[dense] as usize - 1]);
                            sum += near.unwrap_or(Vec3::ZERO);
                        }
                    }
                }
                // A lone voxel has no normal and keeps none.
                Some(if own == Vec3::ZERO {
                    own
                } else {
                    sum.normalize_or(own)
                })
            })
            .collect();
    }
    normals
}

impl Surface {
    /// Finds the surface of `cloud`. With `section`, only the material
    /// behind the plane through the shot line is kept, so that the cut face
    /// shows what is inside.
    #[must_use]
    pub fn extract(cloud: &Cloud, section: bool) -> Self {
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

        let normals = normals_of(&tallies, &slots, dims);
        let patches = tallies
            .iter()
            .zip(&normals)
            .filter_map(|(tally, normal)| {
                Some(Patch {
                    point: tally.mean(),
                    normal: (*normal)?,
                })
            })
            .collect();
        Self {
            patches,
            voxel,
            bounds: (lower, upper),
        }
    }

    /// Range of `field` over the surface, never empty.
    #[must_use]
    pub fn range(&self, field: Field) -> (f64, f64) {
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
pub struct Camera {
    /// Rotation about the vertical axis, in radians.
    pub yaw: f32,
    /// Elevation above the horizontal, in radians.
    pub pitch: f32,
    /// Shift of the model on screen, in pixels: to the right and down.
    pub pan: [f32; 2],
    /// Magnification relative to fitting the model in the panel.
    pub zoom: f32,
}

impl Default for Camera {
    /// A little to the side of and above the section plane.
    fn default() -> Self {
        Self {
            yaw: -0.45,
            pitch: 0.3,
            pan: [0.0; 2],
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// Rotation from model axes to view axes: `x` to the right, `y` up and
    /// `z` towards the viewer.
    #[must_use]
    pub fn rotation(&self) -> Mat3 {
        Mat3::from_rotation_x(self.pitch) * Mat3::from_rotation_y(self.yaw)
    }
}
