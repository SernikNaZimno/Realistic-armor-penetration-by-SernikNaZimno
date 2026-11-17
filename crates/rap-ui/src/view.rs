//! Drawing of the particles: colouring, camera and legend.

use eframe::egui::{self, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use glam::DVec2;
use rap_solver::Particle;

/// Colour stops of the scalar colour map, from low to high (viridis).
const GRADIENT: [[f64; 3]; 5] = [
    [68.0, 1.0, 84.0],
    [59.0, 82.0, 139.0],
    [33.0, 145.0, 140.0],
    [94.0, 201.0, 98.0],
    [253.0, 231.0, 37.0],
];

/// Colours of materials, by material index.
const MATERIAL_COLORS: [[f64; 3]; 6] = [
    [230.0, 145.0, 56.0],
    [120.0, 144.0, 156.0],
    [106.0, 168.0, 79.0],
    [166.0, 77.0, 121.0],
    [61.0, 133.0, 198.0],
    [241.0, 194.0, 50.0],
];

/// Brightness of failed material relative to intact material.
const FAILED_SHADE: f64 = 0.45;
/// Limits of the zoom factor relative to fitting the whole model.
const ZOOM_RANGE: (f32, f32) = (0.2, 80.0);
/// Fraction of the panel the model fills at zoom 1.
const FIT_MARGIN: f32 = 0.94;

/// Quantity shown by the particle colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Material,
    Damage,
    PlasticStrain,
    Pressure,
    Temperature,
    Speed,
}

impl Field {
    pub(crate) const ALL: [Self; 6] = [
        Self::Material,
        Self::Damage,
        Self::PlasticStrain,
        Self::Pressure,
        Self::Temperature,
        Self::Speed,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Material => "Material",
            Self::Damage => "Damage",
            Self::PlasticStrain => "Plastic strain",
            Self::Pressure => "Pressure [GPa]",
            Self::Temperature => "Temperature [K]",
            Self::Speed => "Speed [m/s]",
        }
    }

    /// Value of the field at a particle, in the unit of the label.
    pub(crate) fn value(self, particle: &Particle) -> f64 {
        match self {
            Self::Material => 0.0,
            Self::Damage => particle.damage,
            Self::PlasticStrain => particle.plastic_strain,
            Self::Pressure => particle.pressure * 1.0e-9,
            Self::Temperature => particle.temperature,
            Self::Speed => particle.velocity.length(),
        }
    }

    /// Range of values mapped onto the colour scale.
    pub(crate) fn range(self, particles: &[Particle]) -> (f64, f64) {
        if self == Self::Damage {
            return (0.0, 1.0);
        }
        let (lowest, highest) = particles
            .iter()
            .map(|particle| self.value(particle))
            .fold((f64::MAX, f64::MIN), |(lowest, highest), value| {
                (lowest.min(value), highest.max(value))
            });
        if lowest > highest {
            (0.0, 1.0)
        } else if highest - lowest < 1.0e-12 {
            (lowest, lowest + 1.0)
        } else {
            (lowest, highest)
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to the range of a colour channel"
)]
fn channel(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn color(rgb: [f64; 3], shade: f64) -> Color32 {
    Color32::from_rgb(
        channel(rgb[0] * shade),
        channel(rgb[1] * shade),
        channel(rgb[2] * shade),
    )
}

/// Colour of the scalar colour map at `fraction` of the way from its low
/// to its high end.
pub(crate) fn gradient(fraction: f64) -> Color32 {
    let scaled = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0) * 4.0
    };
    let (index, lower) = if scaled >= 3.0 {
        (3, 3.0)
    } else if scaled >= 2.0 {
        (2, 2.0)
    } else if scaled >= 1.0 {
        (1, 1.0)
    } else {
        (0, 0.0)
    };
    let (from, to) = (GRADIENT[index], GRADIENT[index + 1]);
    let blend = scaled - lower;
    color(
        [
            from[0] + blend * (to[0] - from[0]),
            from[1] + blend * (to[1] - from[1]),
            from[2] + blend * (to[2] - from[2]),
        ],
        1.0,
    )
}

/// Colour of a material; failed material is drawn darker.
pub(crate) fn material_color(material: usize, failed: bool) -> Color32 {
    let shade = if failed { FAILED_SHADE } else { 1.0 };
    color(MATERIAL_COLORS[material % MATERIAL_COLORS.len()], shade)
}

fn particle_color(particle: &Particle, field: Field, range: (f64, f64)) -> Color32 {
    match field {
        Field::Material => material_color(particle.material, particle.is_failed()),
        _ => gradient((field.value(particle) - range.0) / (range.1 - range.0)),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "screen coordinates fit in f32 comfortably"
)]
fn to_f32(value: f64) -> f32 {
    value as f32
}

/// Pan and zoom of the view, relative to showing the whole model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Camera {
    /// Shift of the model on screen, in points.
    pub(crate) pan: Vec2,
    /// Magnification relative to fitting the model in the panel.
    pub(crate) zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

/// What to draw.
pub(crate) struct Scene<'a> {
    pub(crate) particles: &'a [Particle],
    /// Lower and upper corners of the region to fit in the panel, in m.
    pub(crate) bounds: (DVec2, DVec2),
    /// Edge length of the square drawn for each particle, in m.
    pub(crate) particle_size: f64,
    /// `y` of the axis about which an axisymmetric model is mirrored.
    pub(crate) mirror_axis: Option<f64>,
}

/// Mapping from model coordinates to screen points.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Projection {
    centre: DVec2,
    scale: f32,
    origin: Pos2,
}

impl Projection {
    pub(crate) fn new(bounds: (DVec2, DVec2), rect: Rect, camera: Camera) -> Self {
        let size = (bounds.1 - bounds.0).max(DVec2::splat(1.0e-9));
        let fit = (rect.width() / to_f32(size.x)).min(rect.height() / to_f32(size.y));
        Self {
            centre: (bounds.0 + bounds.1) / 2.0,
            scale: fit * FIT_MARGIN * camera.zoom,
            origin: rect.center() + camera.pan,
        }
    }

    /// Screen position of a model point; model `y` points up.
    pub(crate) fn project(&self, point: DVec2) -> Pos2 {
        let offset = point - self.centre;
        self.origin + Vec2::new(to_f32(offset.x), -to_f32(offset.y)) * self.scale
    }
}

/// Builds one mesh with a coloured square per visible particle.
pub(crate) fn particle_mesh(
    scene: &Scene<'_>,
    field: Field,
    projection: &Projection,
    clip: Rect,
) -> Mesh {
    let range = field.range(scene.particles);
    let half = (to_f32(scene.particle_size) * projection.scale / 2.0).max(0.5);
    let visible = clip.expand(half);
    let mut mesh = Mesh::default();
    let mut add = |position: DVec2, color: Color32| {
        let centre = projection.project(position);
        if visible.contains(centre) {
            mesh.add_colored_rect(
                Rect::from_center_size(centre, Vec2::splat(2.0 * half)),
                color,
            );
        }
    };
    for particle in scene.particles {
        let color = particle_color(particle, field, range);
        add(particle.position, color);
        if let Some(axis) = scene.mirror_axis {
            add(
                DVec2::new(particle.position.x, 2.0 * axis - particle.position.y),
                color,
            );
        }
    }
    mesh
}

/// Region shown for a model: its bounds, mirrored about the axis if any.
pub(crate) fn mirrored_bounds(bounds: (DVec2, DVec2), mirror_axis: Option<f64>) -> (DVec2, DVec2) {
    match mirror_axis {
        None => bounds,
        Some(axis) => {
            let reach = (bounds.1.y - axis).max(axis - bounds.0.y);
            (
                DVec2::new(bounds.0.x, axis - reach),
                DVec2::new(bounds.1.x, axis + reach),
            )
        }
    }
}

pub(crate) fn legend(
    painter: &egui::Painter,
    rect: Rect,
    field: Field,
    range: (f64, f64),
    text: Color32,
) {
    const WIDTH: f32 = 160.0;
    const HEIGHT: f32 = 10.0;
    const STEPS: u8 = 32;
    let left = rect.left() + 12.0;
    let top = rect.bottom() - 34.0;
    let font = FontId::proportional(12.0);
    painter.text(
        Pos2::new(left, top - 4.0),
        Align2::LEFT_BOTTOM,
        field.label(),
        font.clone(),
        text,
    );
    if field == Field::Material {
        return;
    }
    let step = WIDTH / f32::from(STEPS);
    for index in 0..STEPS {
        let from = Pos2::new(left + step * f32::from(index), top);
        let color = gradient(f64::from(index) / f64::from(STEPS - 1));
        painter.rect_filled(
            Rect::from_min_size(from, Vec2::new(step + 0.5, HEIGHT)),
            0.0,
            color,
        );
    }
    let below = top + HEIGHT + 2.0;
    painter.text(
        Pos2::new(left, below),
        Align2::LEFT_TOP,
        format!("{:.3}", range.0),
        font.clone(),
        text,
    );
    painter.text(
        Pos2::new(left + WIDTH, below),
        Align2::RIGHT_TOP,
        format!("{:.3}", range.1),
        font,
        text,
    );
}

/// Draws `scene` in the remaining space of `ui` and lets the user drag to
/// pan, scroll to zoom and double-click to reset the camera.
pub(crate) fn show(ui: &mut egui::Ui, scene: &Scene<'_>, field: Field, camera: &mut Camera) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = response.rect;
    painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);

    if response.dragged() {
        camera.pan += response.drag_delta();
    }
    if response.double_clicked() {
        *camera = Camera::default();
    }
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let factor = (scroll * 0.003).exp();
            let zoom = (camera.zoom * factor).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
            // Keep the point under the cursor in place.
            if let Some(cursor) = response.hover_pos() {
                let anchor = cursor - rect.center();
                camera.pan = anchor + (camera.pan - anchor) * (zoom / camera.zoom);
            }
            camera.zoom = zoom;
        }
    }

    let bounds = mirrored_bounds(scene.bounds, scene.mirror_axis);
    let projection = Projection::new(bounds, rect, *camera);
    painter.add(Shape::mesh(particle_mesh(scene, field, &projection, rect)));
    if let Some(axis) = scene.mirror_axis {
        let stroke = Stroke::new(1.0_f32, ui.visuals().weak_text_color());
        let left = projection.project(DVec2::new(bounds.0.x, axis));
        let right = projection.project(DVec2::new(bounds.1.x, axis));
        painter.line_segment([left, right], stroke);
    }
    legend(
        &painter,
        rect,
        field,
        field.range(scene.particles),
        ui.visuals().text_color(),
    );
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Color32, Pos2, Rect, Vec2};
    use glam::{DMat2, DVec2};
    use rap_solver::{Deviator, Particle};

    use super::{
        Camera, Field, Projection, Scene, gradient, material_color, mirrored_bounds, particle_mesh,
    };

    fn particle(x: f64, y: f64, damage: f64) -> Particle {
        Particle {
            position: DVec2::new(x, y),
            velocity: DVec2::new(300.0, 400.0),
            velocity_gradient: DMat2::ZERO,
            mass: 1.0,
            volume: 1.0e-4,
            body: 0,
            material: 1,
            deviator: Deviator::default(),
            pressure: 2.0e9,
            viscous_pressure: 0.0,
            specific_energy: 0.0,
            plastic_strain: 0.25,
            plastic_strain_rate: 0.0,
            temperature: 500.0,
            damage,
            sound_speed: 5000.0,
        }
    }

    #[test]
    fn gradient_runs_from_its_first_to_its_last_stop() {
        assert_eq!(gradient(0.0), Color32::from_rgb(68, 1, 84));
        assert_eq!(gradient(1.0), Color32::from_rgb(253, 231, 37));
        assert_eq!(gradient(0.5), Color32::from_rgb(33, 145, 140));
        // Out-of-range and undefined values stay on the scale.
        assert_eq!(gradient(-3.0), gradient(0.0));
        assert_eq!(gradient(7.0), gradient(1.0));
        assert_eq!(gradient(f64::NAN), gradient(0.0));
    }

    #[test]
    fn failed_material_is_darker_and_colours_repeat() {
        let intact = material_color(1, false);
        let failed = material_color(1, true);
        assert!(failed.r() < intact.r() && failed.g() < intact.g() && failed.b() < intact.b());
        assert_eq!(material_color(0, false), material_color(6, false));
    }

    #[test]
    fn field_values_use_display_units() {
        let sample = particle(0.0, 0.0, 0.5);
        assert!((Field::Pressure.value(&sample) - 2.0).abs() < 1e-12);
        assert!((Field::Speed.value(&sample) - 500.0).abs() < 1e-9);
        assert!((Field::Damage.value(&sample) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn ranges_are_never_empty() {
        assert_eq!(Field::Temperature.range(&[]), (0.0, 1.0));
        let uniform = [particle(0.0, 0.0, 0.0), particle(1.0, 0.0, 0.0)];
        assert_eq!(Field::Temperature.range(&uniform), (500.0, 501.0));
        assert_eq!(Field::Damage.range(&uniform), (0.0, 1.0));
    }

    #[test]
    fn projection_fits_the_model_and_points_y_up() {
        let rect = Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(400.0, 200.0));
        let bounds = (DVec2::new(0.0, 0.0), DVec2::new(2.0, 1.0));
        let projection = Projection::new(bounds, rect, Camera::default());
        let centre = projection.project(DVec2::new(1.0, 0.5));
        assert!((centre - rect.center()).length() < 1e-3);
        let top_right = projection.project(DVec2::new(2.0, 1.0));
        assert!(top_right.x > centre.x && top_right.y < centre.y);
        assert!(rect.contains(top_right));
        // Zoom scales distances from the centre; pan shifts everything.
        let camera = Camera {
            pan: Vec2::new(10.0, -5.0),
            zoom: 2.0,
        };
        let moved = Projection::new(bounds, rect, camera).project(DVec2::new(2.0, 1.0));
        let expected = rect.center() + (top_right - rect.center()) * 2.0 + camera.pan;
        assert!((moved - expected).length() < 1e-3);
    }

    #[test]
    fn mesh_has_one_square_per_visible_particle_and_mirror_image() {
        let particles = [
            particle(0.5, 0.25, 0.0),
            particle(1.5, 0.75, 1.0),
            particle(50.0, 0.5, 0.0),
        ];
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
        let bounds = (DVec2::new(0.0, 0.0), DVec2::new(2.0, 1.0));
        let mut scene = Scene {
            particles: &particles,
            bounds,
            particle_size: 0.05,
            mirror_axis: None,
        };
        let projection = Projection::new(bounds, rect, Camera::default());
        // The particle far outside the panel is culled.
        let mesh = particle_mesh(&scene, Field::Damage, &projection, rect);
        assert_eq!(mesh.vertices.len(), 2 * 4);
        assert_eq!(mesh.vertices[0].color, gradient(0.0));
        assert_eq!(mesh.vertices[4].color, gradient(1.0));

        scene.mirror_axis = Some(0.0);
        let mirrored = mirrored_bounds(bounds, scene.mirror_axis);
        assert_eq!(mirrored, (DVec2::new(0.0, -1.0), DVec2::new(2.0, 1.0)));
        let projection = Projection::new(mirrored, rect, Camera::default());
        let mesh = particle_mesh(&scene, Field::Material, &projection, rect);
        assert_eq!(mesh.vertices.len(), 2 * 2 * 4);
    }
}
