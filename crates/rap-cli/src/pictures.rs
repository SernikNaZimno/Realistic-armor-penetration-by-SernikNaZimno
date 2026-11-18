//! Pictures of a run: one PNG file per snapshot, drawn like the
//! three-dimensional view of the desktop application.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use glam::Vec3;
use rap_app::Simulation;
use rap_render::{Camera, Field, Image, Rgb, Surface, text};

/// Background of the pictures.
const BACKGROUND: Rgb = [255, 255, 255];
/// Colour of the lettering.
const INK: Rgb = [40, 40, 40];
/// Picture height for each pixel of a dot of the lettering.
const HEIGHT_PER_DOT: usize = 360;
/// Most points of the cloud made of an axisymmetric model.
const CLOUD_BUDGET: usize = 4_000_000;
/// Largest picture edge accepted, in pixels.
const MAX_EDGE: usize = 8192;

/// What the colours of the pictures show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Colouring {
    /// Armor in grey, the projectile by material; failed material darker.
    Material,
    /// Accumulated damage.
    Damage,
    /// Equivalent plastic strain.
    Strain,
    /// Pressure.
    Pressure,
    /// Temperature.
    Temperature,
    /// Speed.
    Speed,
}

impl From<Colouring> for Field {
    fn from(colouring: Colouring) -> Self {
        match colouring {
            Colouring::Material => Self::Material,
            Colouring::Damage => Self::Damage,
            Colouring::Strain => Self::PlasticStrain,
            Colouring::Pressure => Self::Pressure,
            Colouring::Temperature => Self::Temperature,
            Colouring::Speed => Self::Speed,
        }
    }
}

/// Two numbers written with a separator between them, such as `1280x720`
/// or `-25,15`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pair<T>(pub(crate) T, pub(crate) T);

impl<T: FromStr> FromStr for Pair<T> {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = text.split(['x', 'X', ',']).map(str::trim).collect();
        let [first, second] = parts.as_slice() else {
            return Err(format!("`{text}` is not two values"));
        };
        let parse = |part: &str| {
            part.parse::<T>()
                .map_err(|_| format!("`{part}` is not a number"))
        };
        Ok(Self(parse(first)?, parse(second)?))
    }
}

/// How the pictures of a run are drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PictureOptions {
    /// What the colours show.
    pub(crate) field: Field,
    /// Where the viewer stands.
    pub(crate) camera: Camera,
    /// Width and height, in pixels.
    pub(crate) size: (usize, usize),
    /// Whether the model is cut along the shot line.
    pub(crate) section: bool,
}

impl PictureOptions {
    /// Options from the command line: `view` is the rotation about the
    /// vertical axis and the elevation, in degrees; `size` the width and
    /// height in pixels.
    ///
    /// # Errors
    /// Fails if the size is zero or unreasonably large.
    pub(crate) fn new(
        colouring: Colouring,
        view: Option<Pair<f32>>,
        size: Pair<usize>,
        whole: bool,
    ) -> Result<Self> {
        let edges = 1..=MAX_EDGE;
        if !edges.contains(&size.0) || !edges.contains(&size.1) {
            bail!(
                "a picture must be between 1 and {MAX_EDGE} pixels each way, not {} by {}",
                size.0,
                size.1
            );
        }
        let mut camera = Camera::default();
        if let Some(Pair(yaw, pitch)) = view {
            camera.yaw = yaw.to_radians();
            camera.pitch = pitch.to_radians();
        }
        Ok(Self {
            field: colouring.into(),
            camera,
            size: (size.0, size.1),
            section: !whole,
        })
    }
}

/// The part of space a series of pictures shows: that of the first one, so
/// that the model does not shift or shrink from frame to frame as debris
/// spreads.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Framing(Option<(Vec3, Vec3)>);

/// Writes the picture of the current state of `simulation` to `path` and
/// returns the range of values its colours span. The lines of `caption`
/// are written in its top left corner, and the scale of the colours, if
/// they show a quantity, in the bottom left. The first picture sets
/// `framing` for those that follow. Returns `None`, and writes nothing, for
/// a model in plane strain.
///
/// # Errors
/// Fails if the file cannot be written.
pub(crate) fn save(
    simulation: &Simulation,
    options: &PictureOptions,
    framing: &mut Framing,
    caption: &[&str],
    path: &Path,
) -> Result<Option<(f64, f64)>> {
    let Some(cloud) = rap_app::picture::cloud(simulation, CLOUD_BUDGET) else {
        return Ok(None);
    };
    let mut surface = Surface::extract(&cloud, options.section);
    surface.bounds = *framing.0.get_or_insert(surface.bounds);
    let mut image = Image::of(
        &surface,
        options.field,
        &options.camera,
        options.size.0,
        options.size.1,
        BACKGROUND,
    );
    let range = surface.range(options.field);
    let dot = (options.size.1 / HEIGHT_PER_DOT).max(1);
    text::caption(&mut image, caption, dot, INK);
    if options.field != Field::Material {
        text::colour_scale(&mut image, options.field.label(), range, dot, INK);
    }
    fs::write(path, image.to_png()?).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(Some(range))
}

#[cfg(test)]
mod tests {
    use rap_render::Field;

    use super::{Colouring, Pair, PictureOptions};

    #[test]
    fn pairs_are_read_with_either_separator() {
        assert_eq!("1280x720".parse(), Ok(Pair(1280_usize, 720)));
        assert_eq!(" -25 , 15.5 ".parse(), Ok(Pair(-25.0_f32, 15.5)));
        assert!("1280".parse::<Pair<usize>>().is_err());
        assert!("1x2x3".parse::<Pair<usize>>().is_err());
        assert!("wide,high".parse::<Pair<f32>>().is_err());
    }

    #[test]
    fn options_take_the_view_in_degrees_and_check_the_size() {
        let options = PictureOptions::new(
            Colouring::Strain,
            Some(Pair(90.0, -30.0)),
            Pair(640, 480),
            true,
        )
        .expect("valid options");
        assert_eq!(options.field, Field::PlasticStrain);
        assert!((options.camera.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((options.camera.pitch + std::f32::consts::FRAC_PI_6).abs() < 1e-6);
        assert!(!options.section);
        let default = PictureOptions::new(Colouring::Material, None, Pair(640, 480), false)
            .expect("valid options");
        assert!(default.section);
        assert!(PictureOptions::new(Colouring::Speed, None, Pair(0, 480), false).is_err());
        assert!(PictureOptions::new(Colouring::Speed, None, Pair(640, 100_000), false).is_err());
    }
}
