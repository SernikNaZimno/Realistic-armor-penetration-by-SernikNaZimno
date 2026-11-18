//! Lettering for pictures: a caption, and the scale of the colours.
//!
//! The letters are those of the common 5 by 7 dot-matrix character set,
//! with an eighth row for descenders, enlarged by a whole factor. That is
//! enough for a time, a depth and the ends of a colour scale, and needs no
//! font file.

use crate::palette::{self, Rgb};
use crate::picture::Image;

/// Columns of a letter.
const COLUMNS: usize = 5;
/// Rows of a letter, descenders included.
const ROWS: usize = 8;
/// Distance from one letter to the next, in dots.
const ADVANCE: usize = COLUMNS + 1;
/// Distance from one line to the next, in dots.
const LINE: usize = ROWS + 3;
/// Distance of the lettering from the edge of the picture, in dots.
const MARGIN: usize = 6;
/// Length of the colour scale, in dots.
const SCALE_LENGTH: usize = 150;
/// Thickness of the colour scale, in dots.
const SCALE_THICKNESS: usize = 8;
/// Gap between the colour scale and the values below it, in dots.
const SCALE_GAP: usize = 2;

/// The printable ASCII characters from the space to the tilde: one byte
/// per column, the lowest bit at the top.
const GLYPHS: [[u8; COLUMNS]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x00, 0x5F, 0x00, 0x00],
    [0x00, 0x07, 0x00, 0x07, 0x00],
    [0x14, 0x7F, 0x14, 0x7F, 0x14],
    [0x24, 0x2A, 0x7F, 0x2A, 0x12],
    [0x23, 0x13, 0x08, 0x64, 0x62],
    [0x36, 0x49, 0x56, 0x20, 0x50],
    [0x00, 0x08, 0x07, 0x03, 0x00],
    [0x00, 0x1C, 0x22, 0x41, 0x00],
    [0x00, 0x41, 0x22, 0x1C, 0x00],
    [0x2A, 0x1C, 0x7F, 0x1C, 0x2A],
    [0x08, 0x08, 0x3E, 0x08, 0x08],
    [0x00, 0x80, 0x70, 0x30, 0x00],
    [0x08, 0x08, 0x08, 0x08, 0x08],
    [0x00, 0x00, 0x60, 0x60, 0x00],
    [0x20, 0x10, 0x08, 0x04, 0x02],
    [0x3E, 0x51, 0x49, 0x45, 0x3E],
    [0x00, 0x42, 0x7F, 0x40, 0x00],
    [0x72, 0x49, 0x49, 0x49, 0x46],
    [0x21, 0x41, 0x49, 0x4D, 0x33],
    [0x18, 0x14, 0x12, 0x7F, 0x10],
    [0x27, 0x45, 0x45, 0x45, 0x39],
    [0x3C, 0x4A, 0x49, 0x49, 0x31],
    [0x41, 0x21, 0x11, 0x09, 0x07],
    [0x36, 0x49, 0x49, 0x49, 0x36],
    [0x46, 0x49, 0x49, 0x29, 0x1E],
    [0x00, 0x00, 0x14, 0x00, 0x00],
    [0x00, 0x40, 0x34, 0x00, 0x00],
    [0x00, 0x08, 0x14, 0x22, 0x41],
    [0x14, 0x14, 0x14, 0x14, 0x14],
    [0x00, 0x41, 0x22, 0x14, 0x08],
    [0x02, 0x01, 0x59, 0x09, 0x06],
    [0x3E, 0x41, 0x5D, 0x59, 0x4E],
    [0x7C, 0x12, 0x11, 0x12, 0x7C],
    [0x7F, 0x49, 0x49, 0x49, 0x36],
    [0x3E, 0x41, 0x41, 0x41, 0x22],
    [0x7F, 0x41, 0x41, 0x41, 0x3E],
    [0x7F, 0x49, 0x49, 0x49, 0x41],
    [0x7F, 0x09, 0x09, 0x09, 0x01],
    [0x3E, 0x41, 0x41, 0x51, 0x73],
    [0x7F, 0x08, 0x08, 0x08, 0x7F],
    [0x00, 0x41, 0x7F, 0x41, 0x00],
    [0x20, 0x40, 0x41, 0x3F, 0x01],
    [0x7F, 0x08, 0x14, 0x22, 0x41],
    [0x7F, 0x40, 0x40, 0x40, 0x40],
    [0x7F, 0x02, 0x1C, 0x02, 0x7F],
    [0x7F, 0x04, 0x08, 0x10, 0x7F],
    [0x3E, 0x41, 0x41, 0x41, 0x3E],
    [0x7F, 0x09, 0x09, 0x09, 0x06],
    [0x3E, 0x41, 0x51, 0x21, 0x5E],
    [0x7F, 0x09, 0x19, 0x29, 0x46],
    [0x26, 0x49, 0x49, 0x49, 0x32],
    [0x03, 0x01, 0x7F, 0x01, 0x03],
    [0x3F, 0x40, 0x40, 0x40, 0x3F],
    [0x1F, 0x20, 0x40, 0x20, 0x1F],
    [0x3F, 0x40, 0x38, 0x40, 0x3F],
    [0x63, 0x14, 0x08, 0x14, 0x63],
    [0x03, 0x04, 0x78, 0x04, 0x03],
    [0x61, 0x59, 0x49, 0x4D, 0x43],
    [0x00, 0x7F, 0x41, 0x41, 0x41],
    [0x02, 0x04, 0x08, 0x10, 0x20],
    [0x00, 0x41, 0x41, 0x41, 0x7F],
    [0x04, 0x02, 0x01, 0x02, 0x04],
    [0x40, 0x40, 0x40, 0x40, 0x40],
    [0x00, 0x03, 0x07, 0x08, 0x00],
    [0x20, 0x54, 0x54, 0x78, 0x40],
    [0x7F, 0x28, 0x44, 0x44, 0x38],
    [0x38, 0x44, 0x44, 0x44, 0x28],
    [0x38, 0x44, 0x44, 0x28, 0x7F],
    [0x38, 0x54, 0x54, 0x54, 0x18],
    [0x00, 0x08, 0x7E, 0x09, 0x02],
    [0x18, 0xA4, 0xA4, 0x9C, 0x78],
    [0x7F, 0x08, 0x04, 0x04, 0x78],
    [0x00, 0x44, 0x7D, 0x40, 0x00],
    [0x20, 0x40, 0x40, 0x3D, 0x00],
    [0x7F, 0x10, 0x28, 0x44, 0x00],
    [0x00, 0x41, 0x7F, 0x40, 0x00],
    [0x7C, 0x04, 0x78, 0x04, 0x78],
    [0x7C, 0x08, 0x04, 0x04, 0x78],
    [0x38, 0x44, 0x44, 0x44, 0x38],
    [0xFC, 0x18, 0x24, 0x24, 0x18],
    [0x18, 0x24, 0x24, 0x18, 0xFC],
    [0x7C, 0x08, 0x04, 0x04, 0x08],
    [0x48, 0x54, 0x54, 0x54, 0x24],
    [0x04, 0x04, 0x3F, 0x44, 0x24],
    [0x3C, 0x40, 0x40, 0x20, 0x7C],
    [0x1C, 0x20, 0x40, 0x20, 0x1C],
    [0x3C, 0x40, 0x30, 0x40, 0x3C],
    [0x44, 0x28, 0x10, 0x28, 0x44],
    [0x4C, 0x90, 0x90, 0x90, 0x7C],
    [0x44, 0x64, 0x54, 0x4C, 0x44],
    [0x00, 0x08, 0x36, 0x41, 0x00],
    [0x00, 0x00, 0x77, 0x00, 0x00],
    [0x00, 0x41, 0x36, 0x08, 0x00],
    [0x02, 0x01, 0x02, 0x04, 0x02],
];
/// The micro sign: a `u` with a stem below the line.
const MICRO: [u8; COLUMNS] = [0xFC, 0x40, 0x40, 0x20, 0x7C];
/// The degree sign.
const DEGREE: [u8; COLUMNS] = [0x00, 0x06, 0x09, 0x09, 0x06];

/// Dots of `character`; those of a question mark if it has none.
fn glyph(character: char) -> [u8; COLUMNS] {
    match character {
        'µ' | 'μ' => MICRO,
        '°' => DEGREE,
        ' '..='~' => GLYPHS[character as usize - ' ' as usize],
        _ => GLYPHS['?' as usize - ' ' as usize],
    }
}

/// Width of `text` at `scale` pixels per dot, in pixels.
#[must_use]
pub fn width(text: &str, scale: usize) -> usize {
    (text.chars().count() * ADVANCE).saturating_sub(1) * scale
}

/// Writes `text` in `ink` with its top left corner `x` from the left and
/// `y` from the top of `image`, at `scale` pixels per dot. What falls
/// outside the picture is left out.
pub fn draw(image: &mut Image, x: usize, y: usize, text: &str, scale: usize, ink: Rgb) {
    for (index, character) in text.chars().enumerate() {
        let left = x + index * ADVANCE * scale;
        for (column, dots) in glyph(character).into_iter().enumerate() {
            for row in (0..ROWS).filter(|row| (dots >> row) & 1 == 1) {
                image.fill(left + column * scale, y + row * scale, scale, scale, ink);
            }
        }
    }
}

/// Writes `lines` one below the other in the top left corner of `image`.
pub fn caption(image: &mut Image, lines: &[&str], scale: usize, ink: Rgb) {
    for (index, line) in lines.iter().enumerate() {
        let top = (MARGIN + index * LINE) * scale;
        draw(image, MARGIN * scale, top, line, scale, ink);
    }
}

/// A value of a colour scale written with three digits or so.
#[must_use]
pub fn number(value: f64) -> String {
    let size = value.abs();
    if size >= 100.0 {
        format!("{value:.0}")
    } else if size >= 10.0 {
        format!("{value:.1}")
    } else if size >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

/// Draws the scale of the colours in the bottom left corner of `image`:
/// `label` above a strip of the gradient, and the two ends of `range`
/// below it.
pub fn colour_scale(image: &mut Image, label: &str, range: (f64, f64), scale: usize, ink: Rgb) {
    let (_, height) = image.size();
    let left = MARGIN * scale;
    let length = SCALE_LENGTH * scale;
    let values = height.saturating_sub((MARGIN + ROWS) * scale);
    let strip = values.saturating_sub((SCALE_THICKNESS + SCALE_GAP) * scale);
    let title = strip.saturating_sub(LINE * scale);
    draw(image, left, title, label, scale, ink);
    #[expect(
        clippy::cast_precision_loss,
        reason = "a colour scale is far shorter than 2^52 pixels"
    )]
    let last = (length - 1).max(1) as f64;
    for offset in 0..length {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a colour scale is far shorter than 2^52 pixels"
        )]
        let fraction = offset as f64 / last;
        let colour = palette::gradient(fraction);
        image.fill(left + offset, strip, 1, SCALE_THICKNESS * scale, colour);
    }
    let highest = number(range.1);
    draw(image, left, values, &number(range.0), scale, ink);
    let right = (left + length).saturating_sub(width(&highest, scale));
    draw(image, right, values, &highest, scale, ink);
}

#[cfg(test)]
mod tests {
    use super::{caption, colour_scale, draw, number, width};
    use crate::palette::{self, Rgb};
    use crate::picture::Image;

    const PAPER: Rgb = [255, 255, 255];
    const INK: Rgb = [10, 20, 30];

    #[test]
    fn a_letter_is_drawn_dot_by_dot_at_the_scale_asked_for() {
        // A capital I: a stem in the middle column with a serif at each end.
        let mut image = Image::new(12, 18, PAPER);
        draw(&mut image, 1, 1, "I", 2, INK);
        for row in 0..14 {
            assert_eq!(image.pixel(5, 1 + row), INK, "stem, row {row}");
            assert_eq!(image.pixel(6, 1 + row), INK, "stem, row {row}");
            assert_eq!(image.pixel(1, 1 + row), PAPER, "first column, row {row}");
        }
        assert_eq!(image.pixel(3, 1), INK);
        assert_eq!(image.pixel(8, 14), INK);
        assert_eq!(image.pixel(3, 7), PAPER);
        // Nothing below the seven rows of a letter without a descender.
        assert_eq!(image.pixel(5, 15), PAPER);
        assert_eq!(image.pixel(5, 0), PAPER);
    }

    #[test]
    fn a_descender_reaches_the_eighth_row() {
        let mut image = Image::new(6, 8, PAPER);
        draw(&mut image, 0, 0, "µ", 1, INK);
        assert_eq!(image.pixel(0, 7), INK);
        assert_eq!(image.pixel(4, 7), PAPER);
    }

    #[test]
    fn unknown_characters_become_question_marks() {
        let mut unknown = Image::new(6, 8, PAPER);
        let mut question = Image::new(6, 8, PAPER);
        draw(&mut unknown, 0, 0, "ż", 1, INK);
        draw(&mut question, 0, 0, "?", 1, INK);
        assert_eq!(unknown, question);
        assert_ne!(unknown, Image::new(6, 8, PAPER));
    }

    #[test]
    fn text_outside_the_picture_is_left_out() {
        let mut image = Image::new(8, 8, PAPER);
        draw(&mut image, 5, 5, "WWWW", 3, INK);
        draw(&mut image, 100, 100, "W", 1, INK);
        assert_eq!(image.pixel(0, 0), PAPER);
        assert_eq!(width("WWWW", 3), 69);
        assert_eq!(width("", 3), 0);
    }

    #[test]
    fn caption_lines_follow_one_another() {
        let mut image = Image::new(200, 60, PAPER);
        caption(&mut image, &["t = 400 µs", "depth 114.8 mm"], 1, INK);
        let inked = |top: usize, bottom: usize| {
            (top..bottom).any(|y| (0..200).any(|x| image.pixel(x, y) == INK))
        };
        assert!(!inked(0, 6));
        assert!(inked(6, 14));
        assert!(!inked(14, 17));
        assert!(inked(17, 25));
        assert!(!inked(26, 60));
    }

    #[test]
    fn the_colour_scale_runs_from_one_end_of_the_gradient_to_the_other() {
        let mut image = Image::new(400, 120, PAPER);
        colour_scale(&mut image, "Pressure [GPa]", (-1.5, 12.0), 2, INK);
        // The strip is 16 pixels thick; its top is 24 dots above the
        // bottom edge.
        let y = 120 - 24 * 2 + 8;
        assert_eq!(image.pixel(12, y), palette::gradient(0.0));
        assert_eq!(image.pixel(12 + 299, y), palette::gradient(1.0));
        assert_eq!(image.pixel(12 + 300, y), PAPER);
        assert_eq!(image.pixel(11, y), PAPER);
    }

    #[test]
    fn numbers_keep_about_three_digits() {
        assert_eq!(number(1234.6), "1235");
        assert_eq!(number(-56.78), "-56.8");
        assert_eq!(number(2.71849), "2.72");
        assert_eq!(number(0.0), "0.000");
        assert_eq!(number(-0.0421), "-0.042");
    }
}
