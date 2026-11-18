//! Pictures of a simulation, shared by the desktop application and the
//! command line: what the colours show ([`palette`]), the surface of the
//! particles of a solid model and the camera it is seen through
//! ([`surface`]), the picture itself ([`picture`]) and the lettering on it
//! ([`text`]).

pub mod palette;
pub mod picture;
pub mod surface;
pub mod text;

pub use palette::{Field, Rgb};
pub use picture::{Image, PictureError, Sprite, Sprites, sprites};
pub use surface::{Camera, Cloud, CloudPoint, Patch, Surface};
