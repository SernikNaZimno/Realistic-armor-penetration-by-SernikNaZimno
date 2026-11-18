//! Turning a surface into a picture: lit patches in drawing order, and an
//! image that can be written as a PNG file.
//!
//! Patches are painted from the farthest to the nearest, so nearer material
//! covers what is behind it.

use glam::Vec3;
use thiserror::Error;

use crate::palette::{self, Field, Rgb};
use crate::surface::{Camera, Patch, Surface};

/// Patch edge in voxel edges; patches overlap so that no gap shows between
/// them.
const PATCH_VOXELS: f32 = 1.7;
/// Brightness of a patch facing away from the light.
const AMBIENT: f32 = 0.32;
/// Brightness of a lone voxel, which has no normal to light.
const UNLIT: f32 = 0.8;
/// Direction from the model to the light, in view space: above the viewer's
/// left shoulder.
const LIGHT: Vec3 = Vec3::new(-0.42, 0.55, 0.72);
/// Patches turned further away from the viewer than this are behind the
/// body they belong to and are not drawn.
const BACK_FACE: f32 = -0.2;
/// Fraction of the picture the model fills at zoom 1.
const FIT_MARGIN: f32 = 0.9;
/// Smallest half-edge of a patch, in pixels.
const MIN_HALF: f32 = 0.5;

/// One patch as it appears in a picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite {
    /// Centre, in pixels from the left edge of the picture.
    pub x: f32,
    /// Centre, in pixels from the top edge of the picture.
    pub y: f32,
    /// Colour, with lighting applied.
    pub color: Rgb,
}

/// The patches of a surface as squares in a picture, farthest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Sprites {
    /// Half the edge of every square, in pixels.
    pub half: f32,
    /// The squares, in drawing order.
    pub items: Vec<Sprite>,
}

fn base_color(patch: &Patch, field: Field, range: (f64, f64)) -> Rgb {
    let point = &patch.point;
    match field {
        Field::Material if point.armor => palette::armor(point.material, point.failed > 0.5),
        Field::Material => palette::material(point.material, point.failed > 0.5),
        _ => palette::gradient((f64::from(point.value(field)) - range.0) / (range.1 - range.0)),
    }
}

/// Places the patches of `surface` that face the viewer in a picture of
/// `width` by `height` pixels, coloured by `field` and lit.
#[must_use]
pub fn sprites(
    surface: &Surface,
    field: Field,
    camera: &Camera,
    width: f32,
    height: f32,
) -> Sprites {
    let rotation = camera.rotation();
    let centre = (surface.bounds.0 + surface.bounds.1) / 2.0;
    let reach = (surface.bounds.1 - surface.bounds.0).length().max(1.0e-9);
    let scale = width.min(height) / reach * FIT_MARGIN * camera.zoom;
    let origin = [width / 2.0 + camera.pan[0], height / 2.0 + camera.pan[1]];
    let half = (PATCH_VOXELS * surface.voxel * scale / 2.0).max(MIN_HALF);
    let range = surface.range(field);
    let light = LIGHT.normalize();

    let mut drawn: Vec<(f32, Sprite)> = surface
        .patches
        .iter()
        .filter_map(|patch| {
            let normal = rotation * patch.normal;
            if normal.z < BACK_FACE {
                return None;
            }
            let seen = rotation * (patch.point.position - centre);
            let (x, y) = (origin[0] + seen.x * scale, origin[1] - seen.y * scale);
            let inside = x > -half && x < width + half && y > -half && y < height + half;
            if !inside {
                return None;
            }
            let lit = if patch.normal == Vec3::ZERO {
                UNLIT
            } else {
                AMBIENT + (1.0 - AMBIENT) * normal.dot(light).max(0.0)
            };
            let color = palette::shaded(base_color(patch, field, range), f64::from(lit));
            Some((seen.z, Sprite { x, y, color }))
        })
        .collect();
    drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
    Sprites {
        half,
        items: drawn.into_iter().map(|(_, sprite)| sprite).collect(),
    }
}

/// Reasons a picture cannot be written.
#[derive(Debug, Error)]
pub enum PictureError {
    /// The picture has no pixels or more than a PNG file can hold.
    #[error("a picture of {width} by {height} pixels cannot be written")]
    Size {
        /// Width asked for, in pixels.
        width: usize,
        /// Height asked for, in pixels.
        height: usize,
    },
    /// The encoder failed.
    #[error("the picture could not be encoded: {0}")]
    Encoding(#[from] png::EncodingError),
}

/// A picture as rows of red, green and blue bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to the picture before it is converted"
)]
fn pixel_index(value: f32, limit: usize) -> usize {
    #[expect(
        clippy::cast_precision_loss,
        reason = "pictures are far smaller than 2^24 pixels across"
    )]
    let limit = limit as f32;
    value.round().clamp(0.0, limit) as usize
}

impl Image {
    /// A picture of `width` by `height` pixels filled with `background`.
    #[must_use]
    pub fn new(width: usize, height: usize, background: Rgb) -> Self {
        Self {
            width,
            height,
            pixels: background.repeat(width * height),
        }
    }

    /// Width and height, in pixels.
    #[must_use]
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Colour of the pixel `x` from the left and `y` from the top.
    ///
    /// # Panics
    /// Panics if the pixel is outside the picture.
    #[must_use]
    pub fn pixel(&self, x: usize, y: usize) -> Rgb {
        assert!(
            x < self.width && y < self.height,
            "pixel outside the picture"
        );
        let start = 3 * (x + y * self.width);
        [
            self.pixels[start],
            self.pixels[start + 1],
            self.pixels[start + 2],
        ]
    }

    /// Fills the rectangle of `width` by `height` pixels whose top left
    /// corner is `x` from the left and `y` from the top with `colour`. The
    /// part of it outside the picture is left out.
    pub fn fill(&mut self, x: usize, y: usize, width: usize, height: usize, colour: Rgb) {
        let right = x.saturating_add(width).min(self.width);
        let bottom = y.saturating_add(height).min(self.height);
        if x >= right {
            return;
        }
        let row = colour.repeat(right - x);
        for y in y..bottom {
            let start = 3 * (x + y * self.width);
            self.pixels[start..start + row.len()].copy_from_slice(&row);
        }
    }

    /// Paints `sprites` over the picture, in their order.
    pub fn paint(&mut self, sprites: &Sprites) {
        for sprite in &sprites.items {
            let left = pixel_index(sprite.x - sprites.half, self.width);
            let right = pixel_index(sprite.x + sprites.half, self.width);
            let top = pixel_index(sprite.y - sprites.half, self.height);
            let bottom = pixel_index(sprite.y + sprites.half, self.height);
            if right <= left {
                continue;
            }
            let row = sprite.color.repeat(right - left);
            for y in top..bottom {
                let start = 3 * (left + y * self.width);
                self.pixels[start..start + row.len()].copy_from_slice(&row);
            }
        }
    }

    /// The picture of `surface` coloured by `field`, seen through `camera`.
    #[must_use]
    pub fn of(
        surface: &Surface,
        field: Field,
        camera: &Camera,
        width: usize,
        height: usize,
        background: Rgb,
    ) -> Self {
        let mut image = Self::new(width, height, background);
        #[expect(
            clippy::cast_precision_loss,
            reason = "pictures are far smaller than 2^24 pixels across"
        )]
        let size = (width as f32, height as f32);
        image.paint(&sprites(surface, field, camera, size.0, size.1));
        image
    }

    /// The picture as the contents of a PNG file.
    ///
    /// # Errors
    /// Returns [`PictureError`] if the picture is empty or too large, or
    /// the encoder fails.
    pub fn to_png(&self) -> Result<Vec<u8>, PictureError> {
        let size = || PictureError::Size {
            width: self.width,
            height: self.height,
        };
        if self.pixels.is_empty() {
            return Err(size());
        }
        let width = u32::try_from(self.width).map_err(|_| size())?;
        let height = u32::try_from(self.height).map_err(|_| size())?;
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&self.pixels)?;
        writer.finish()?;
        Ok(bytes)
    }
}
