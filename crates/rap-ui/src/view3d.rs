//! Three-dimensional picture of a solid model in the window: the surface
//! found and lit by `rap-render`, drawn as one mesh, with a camera the
//! pointer turns.

use eframe::egui::{self, Color32, Mesh, Rect, Sense, Shape, Vec2};
pub(crate) use rap_render::{Camera as Camera3, Cloud, Surface};

use crate::view::{self, Field};

/// Radians of rotation per point of pointer movement.
const ORBIT_SPEED: f32 = 0.008;
/// Steepest view from above or below, in radians.
const PITCH_LIMIT: f32 = 1.5;
/// Limits of the zoom factor relative to fitting the whole model.
const ZOOM_RANGE: (f32, f32) = (0.2, 80.0);

/// Builds one mesh with a shaded square per patch that faces the viewer,
/// farthest first, inside `rect`.
pub(crate) fn surface_mesh(surface: &Surface, field: Field, camera: &Camera3, rect: Rect) -> Mesh {
    let sprites = rap_render::sprites(surface, field, camera, rect.width(), rect.height());
    let size = Vec2::splat(2.0 * sprites.half);
    let mut mesh = Mesh::default();
    for sprite in &sprites.items {
        let [red, green, blue] = sprite.color;
        mesh.add_colored_rect(
            Rect::from_center_size(rect.min + Vec2::new(sprite.x, sprite.y), size),
            Color32::from_rgb(red, green, blue),
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

    let delta = response.drag_delta();
    if response.dragged_by(egui::PointerButton::Primary) {
        camera.yaw += delta.x * ORBIT_SPEED;
        camera.pitch = (camera.pitch + delta.y * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    } else if response.dragged() {
        camera.pan = [camera.pan[0] + delta.x, camera.pan[1] + delta.y];
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
    use glam::{DMat3, DVec3};
    use rap_solver::Particle3;

    use super::{Camera3, Cloud, Surface, surface_mesh};
    use crate::view::Field;

    fn particle(position: DVec3) -> Particle3 {
        Particle3 {
            position,
            velocity: DVec3::ZERO,
            velocity_gradient: DMat3::ZERO,
            mass: 7.83e-6,
            volume: 1.0e-9,
            body: 0,
            material: 0,
            deviator: DMat3::ZERO,
            pressure: 0.0,
            viscous_pressure: 0.0,
            specific_energy: 0.0,
            plastic_strain: 0.0,
            plastic_strain_rate: 0.0,
            temperature: 294.0,
            damage: 0.0,
            sound_speed: 5000.0,
        }
    }

    #[test]
    fn mesh_has_one_square_per_sprite_inside_the_panel() {
        let particles: Vec<_> = (0..1000_u32)
            .map(|index| {
                let cell = DVec3::new(
                    f64::from(index % 10),
                    f64::from(index / 10 % 10),
                    f64::from(index / 100),
                );
                particle((cell + 0.5) * 1.0e-3)
            })
            .collect();
        let surface = Surface::extract(&Cloud::new(&particles, 1.0, false, 1), false);
        let rect = Rect::from_min_max(Pos2::new(200.0, 100.0), Pos2::new(600.0, 400.0));
        let camera = Camera3::default();
        let mesh = surface_mesh(&surface, Field::Material, &camera, rect);
        let sprites = rap_render::sprites(&surface, Field::Material, &camera, 400.0, 300.0);
        assert!(sprites.items.len() > 100);
        assert_eq!(mesh.vertices.len(), 4 * sprites.items.len());
        // Sprites are placed relative to the panel, not the window.
        let first = sprites.items[0];
        let expected = Pos2::new(
            200.0 + first.x - sprites.half,
            100.0 + first.y - sprites.half,
        );
        assert!((mesh.vertices[0].pos - expected).length() < 1e-3);
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| rect.expand(20.0).contains(vertex.pos))
        );
    }
}
