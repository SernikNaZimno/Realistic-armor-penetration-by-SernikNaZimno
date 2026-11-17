//! The surface of a cloud of particles and the pictures made of it.

use glam::{DMat3, DVec3, Vec3};
use rap_render::{Camera, Cloud, Field, Image, Surface, sprites};
use rap_solver::Particle3;

const SPACING: f64 = 1.0e-3;
const BACKGROUND: [u8; 3] = [10, 20, 30];

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

fn head_on() -> Camera {
    Camera {
        yaw: 0.0,
        pitch: 0.0,
        ..Camera::default()
    }
}

#[test]
fn cloud_keeps_state_in_display_units_and_mirrors_a_half_model() {
    let particles = [particle(DVec3::new(0.01, 0.02, 0.013), 2, 1.0)];
    let whole = Cloud::new(&particles, 0.01, false, 0);
    let point = whole.points[0];
    assert!((point.position - Vec3::new(0.01, 0.02, 0.013)).length() < 1e-7);
    assert_eq!(point.material, 2);
    assert!(point.armor && !Cloud::new(&particles, 0.01, false, 1).points[0].armor);
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
    let cloud = Cloud::new(&block(DVec3::ZERO, [30, 30, 30], 0), 1.0, false, 0);
    let surface = Surface::extract(&cloud, false);
    // Voxels of 1.5 mm: 20 along each edge, of which the skin is kept.
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
    let cut = Surface::extract(&cloud, true);
    assert!(cut.bounds.1.z < 0.0065);
    let on_the_cut = |material: usize| {
        cut.patches
            .iter()
            .filter(|patch| patch.point.material == material && patch.point.position.z > 0.004)
            .count()
    };
    assert!(on_the_cut(1) > 0 && on_the_cut(0) > on_the_cut(1));
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
    let lone = Cloud::new(&[particle(DVec3::splat(0.01), 0, 1.0)], 1.0, false, 1);
    let surface = Surface::extract(&lone, false);
    assert_eq!(surface.patches.len(), 1);
    assert_eq!(surface.patches[0].normal, Vec3::ZERO);
    assert_eq!(surface.range(Field::Damage), (0.0, 1.0));
    assert_eq!(
        sprites(&surface, Field::Material, &Camera::default(), 400.0, 300.0)
            .items
            .len(),
        1
    );

    let empty = Surface::extract(&Cloud::new(&[], 0.0, false, 0), true);
    assert_eq!(empty.patches, Vec::new());
    assert_eq!(empty.range(Field::Speed), (0.0, 1.0));
    let image = Image::of(&empty, Field::Speed, &Camera::default(), 40, 30, BACKGROUND);
    assert_eq!(image.pixel(20, 15), BACKGROUND);
}

#[test]
fn far_side_is_not_drawn_and_nearer_patches_come_last() {
    let cloud = Cloud::new(&block(DVec3::ZERO, [30, 30, 30], 0), 1.0, false, 1);
    let surface = Surface::extract(&cloud, false);
    let drawn = sprites(&surface, Field::Material, &head_on(), 400.0, 300.0);
    // Head on, the near face is drawn and the far face is not.
    let count = drawn.items.len();
    assert!(
        count >= 20 * 20 && count <= surface.patches.len() - 20 * 20,
        "{count}"
    );
    // The last sprites are on the near face, which is lit evenly away
    // from its edges.
    let typical = drawn.items[count - 150].color;
    let near_face = drawn
        .items
        .iter()
        .rev()
        .take(300)
        .filter(|sprite| sprite.color == typical)
        .count();
    assert!(near_face > 200, "{near_face}");
}

#[test]
fn patches_turned_towards_the_light_are_brighter() {
    // Two patches side by side, one facing left and one facing right; the
    // light comes from the viewer's left.
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
    let drawn = sprites(&surface, Field::Material, &head_on(), 400.0, 300.0);
    assert_eq!(drawn.items.len(), 2);
    let (first, second) = (drawn.items[0], drawn.items[1]);
    let (left, right) = if first.x < second.x {
        (first, second)
    } else {
        (second, first)
    };
    assert!(left.color[0] > right.color[0] + 40);
}

#[test]
fn picture_shows_the_model_in_the_middle_and_survives_png() {
    // Armor is grey; a projectile of the same material is not.
    let mut particles = block(DVec3::ZERO, [20, 20, 20], 0);
    for particle in &mut particles {
        particle.body = 1;
    }
    let cloud = Cloud::new(&particles, 1.0, false, 1);
    let surface = Surface::extract(&cloud, false);
    let image = Image::of(&surface, Field::Material, &head_on(), 120, 80, BACKGROUND);
    assert_eq!(image.size(), (120, 80));
    let middle = image.pixel(60, 40);
    assert!(middle[0] == middle[1] && middle[1] == middle[2] && middle[0] > 60);
    assert_eq!(image.pixel(2, 2), BACKGROUND);
    assert_eq!(image.pixel(117, 77), BACKGROUND);

    let bytes = image.to_png().expect("a picture with pixels");
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("a PNG file");
    let mut pixels = vec![0; reader.output_buffer_size().expect("a small picture")];
    let info = reader.next_frame(&mut pixels).expect("one frame");
    assert_eq!((info.width, info.height), (120, 80));
    let start = 3 * (60 + 40 * 120);
    assert_eq!(pixels[start..start + 3], middle);

    assert!(Image::new(0, 10, BACKGROUND).to_png().is_err());
}

/// The picture of an axisymmetric bar: a rectangle of particles from the
/// axis at `y = 0.01` out to `radius`, `length` long.
fn bar(radius: f64, length: f64) -> Vec<rap_solver::Particle> {
    use glam::{DMat2, DVec2};
    let mut particles = Vec::new();
    let (rows, columns) = ((radius / SPACING).round(), (length / SPACING).round());
    let mut row = 0.0;
    while row < rows {
        let mut column = 0.0;
        while column < columns {
            let from_axis = (row + 0.5) * SPACING;
            particles.push(rap_solver::Particle {
                position: DVec2::new((column + 0.5) * SPACING, 0.01 + from_axis),
                velocity: DVec2::new(100.0, 0.0),
                velocity_gradient: DMat2::ZERO,
                mass: 7830.0 * SPACING * SPACING * from_axis,
                volume: SPACING * SPACING * from_axis,
                body: 0,
                material: 0,
                deviator: rap_solver::Deviator::default(),
                pressure: 0.0,
                viscous_pressure: 0.0,
                specific_energy: 0.0,
                plastic_strain: 0.0,
                plastic_strain_rate: 0.0,
                temperature: 294.0,
                damage: 0.0,
                sound_speed: 5000.0,
            });
            column += 1.0;
        }
        row += 1.0;
    }
    particles
}

#[test]
fn axisymmetric_model_is_swept_into_half_a_solid_of_revolution() {
    let (radius, length) = (0.012, 0.03);
    let particles = bar(radius, length);
    let cloud = Cloud::revolved(&particles, 0.01, 1, 10_000_000);
    assert!((cloud.spacing - 1.0e-3).abs() < 1e-6);
    assert!(cloud.plane.abs() < f32::EPSILON);
    // Every point is behind the section plane and within the bar.
    for point in &cloud.points {
        let from_axis = (point.position.y - 0.01).hypot(point.position.z);
        assert!(point.position.z <= 0.0 && f64::from(from_axis) < radius);
        assert!(!point.armor && (point.speed - 100.0).abs() < 1e-3);
    }
    // The points fill half a cylinder about as densely as a lattice would.
    let expected = std::f64::consts::PI * radius * radius / 2.0 * length / (1.2 * SPACING.powi(3));
    let count = f64::from(u32::try_from(cloud.points.len()).expect("a small cloud"));
    assert!(
        (count / expected - 1.0).abs() < 0.15,
        "{count} against {expected}"
    );

    // Its section is the flat face, facing the viewer.
    let surface = Surface::extract(&cloud, true);
    let face: Vec<_> = surface
        .patches
        .iter()
        .filter(|patch| patch.point.position.z > -1.0e-3)
        .filter(|patch| {
            let position = patch.point.position;
            (position.y - 0.01).abs() < 0.008 && position.x > 0.005 && position.x < 0.025
        })
        .collect();
    assert!(face.len() > 50, "{}", face.len());
    assert!(
        face.iter()
            .all(|patch| (patch.normal - Vec3::Z).length() < 1e-3)
    );
}

#[test]
fn a_budget_of_points_makes_a_revolved_cloud_coarser() {
    let particles = bar(0.012, 0.03);
    let full = Cloud::revolved(&particles, 0.01, 1, 10_000_000);
    let limited = Cloud::revolved(&particles, 0.01, 1, 2_000);
    assert!(limited.points.len() < full.points.len() / 2);
    // One copy of each particle at least, so the budget is only a guide.
    assert!(limited.points.len() >= particles.len());
    assert!(limited.spacing > 1.5 * full.spacing);
    assert_eq!(Cloud::revolved(&[], 0.0, 0, 100).points, Vec::new());
}
