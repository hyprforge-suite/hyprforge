//! Turning the picture on screen, on top of the turn its EXIF asked for.
//!
//! Two rotations reach the same photograph and only one of them is the
//! user's: the file says which way up it was taken, and the user may then
//! turn it. Composing them in one place is the point of this module —
//! "the picture is sideways" then has exactly one function to be wrong
//! in, rather than two that can disagree.
//!
//! Nothing here writes to the file. A rotation that is saved is an edit
//! of somebody's original, which is a different decision with a different
//! standard of care (and an undo).

use hyprforge_image::Orientation;

/// Quarter turns clockwise, as the user has asked for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Turns(u8);

impl Turns {
    pub fn none() -> Turns {
        Turns(0)
    }

    pub fn right(self) -> Turns {
        Turns((self.0 + 1) % 4)
    }

    pub fn left(self) -> Turns {
        Turns((self.0 + 3) % 4)
    }

    /// Quarter turns clockwise, 0 to 3.
    pub fn quarters(self) -> u8 {
        self.0
    }

    /// Degrees clockwise, for a renderer that wants an angle.
    pub fn degrees(self) -> f32 {
        f32::from(self.0) * 90.0
    }

    /// Whether this turn swaps the picture's width and height.
    pub fn swaps_axes(self) -> bool {
        self.0 % 2 == 1
    }
}

/// The size the picture presents, with both the file's orientation and
/// the user's turns applied.
///
/// Takes the **already-oriented** size, because that is what
/// `hyprforge-image` hands back — the decoder has applied the EXIF turn
/// before anything here sees it. Passing the raw stored size would apply
/// the file's orientation twice, which is the double-rotation this whole
/// arrangement exists to avoid.
pub fn presented_size(oriented: (u32, u32), turns: Turns) -> (u32, u32) {
    if turns.swaps_axes() {
        (oriented.1, oriented.0)
    } else {
        oriented
    }
}

/// Turns decoded RGBA8 pixels clockwise by `turns`, returning the new
/// pixels and their size.
///
/// The user's turn is applied to the pixels rather than asked of the
/// renderer: iced's `Rotation` turns a picture inside the box it was laid
/// out in, so a quarter turn of a landscape picture draws it cropped to a
/// landscape box. Turning the pixels makes the presented size the real
/// size, which is what every zoom and pan calculation already assumes.
/// The buffer is window-sized (see `hyprforge_image`'s budget), so
/// turning it is a copy of at most that, done once per turn.
pub fn rotate_rgba(pixels: &[u8], width: u32, height: u32, turns: Turns) -> (Vec<u8>, u32, u32) {
    let (w, h) = (width as usize, height as usize);
    if turns.quarters() == 0 || pixels.len() != w * h * 4 {
        return (pixels.to_vec(), width, height);
    }
    let (out_w, out_h) = if turns.swaps_axes() { (h, w) } else { (w, h) };
    let mut out = vec![0u8; pixels.len()];
    for y in 0..h {
        for x in 0..w {
            // Where (x, y) lands after the turn, clockwise.
            let (nx, ny) = match turns.quarters() {
                1 => (h - 1 - y, x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            let from = (y * w + x) * 4;
            let to = (ny * out_w + nx) * 4;
            out[to..to + 4].copy_from_slice(&pixels[from..from + 4]);
        }
    }
    (out, out_w as u32, out_h as u32)
}

/// Whether the picture on screen is the right way up compared to how its
/// pixels are stored — for an info panel that wants to say so.
pub fn is_turned(file: Orientation, turns: Turns) -> bool {
    file != Orientation::Upright || turns != Turns::none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_turns_in_either_direction_come_back_to_the_start() {
        let mut right = Turns::none();
        let mut left = Turns::none();
        for _ in 0..4 {
            right = right.right();
            left = left.left();
        }
        assert_eq!(right, Turns::none());
        assert_eq!(left, Turns::none());
    }

    #[test]
    fn turning_one_way_and_back_is_where_it_started() {
        assert_eq!(Turns::none().right().left(), Turns::none());
        assert_eq!(Turns::none().left().right(), Turns::none());
    }

    #[test]
    fn a_quarter_turn_stands_a_landscape_picture_up() {
        assert_eq!(presented_size((4000, 3000), Turns::none()), (4000, 3000));
        assert_eq!(presented_size((4000, 3000), Turns::none().right()), (3000, 4000));
        assert_eq!(presented_size((4000, 3000), Turns::none().right().right()), (4000, 3000));
    }

    /// The trap this module exists to prevent: the size handed in is the
    /// one the decoder already oriented, so a photograph a phone tagged
    /// sideways is *not* turned a second time here.
    #[test]
    fn the_files_own_orientation_is_not_applied_twice() {
        // A phone photo: stored 4032x3024, tagged Rotate90, so
        // hyprforge-image hands back 3024x4032 already upright.
        let oriented = Orientation::Rotate90.applied_size(4032, 3024);
        assert_eq!(oriented, (3024, 4032));
        // With no user turn, that is what is presented — unchanged.
        assert_eq!(presented_size(oriented, Turns::none()), (3024, 4032));
    }

    #[test]
    fn degrees_match_the_quarter_turns() {
        assert_eq!(Turns::none().degrees(), 0.0);
        assert_eq!(Turns::none().right().degrees(), 90.0);
        assert_eq!(Turns::none().left().degrees(), 270.0);
    }

    #[test]
    fn an_untouched_upright_picture_is_not_reported_as_turned() {
        assert!(!is_turned(Orientation::Upright, Turns::none()));
        assert!(is_turned(Orientation::Rotate90, Turns::none()));
        assert!(is_turned(Orientation::Upright, Turns::none().right()));
    }

    /// A 2x1 picture, red then green. The corner that was top-left is
    /// where a clockwise turn has to put it — the same check the plan
    /// asks of EXIF orientation, for the user's own turn.
    #[test]
    fn turning_the_pixels_right_puts_the_top_left_corner_at_the_top_right() {
        const R: [u8; 4] = [255, 0, 0, 255];
        const G: [u8; 4] = [0, 255, 0, 255];
        let pixels = [R, G].concat();

        let (right, w, h) = rotate_rgba(&pixels, 2, 1, Turns::none().right());
        assert_eq!((w, h), (1, 2));
        assert_eq!(right, [R, G].concat(), "red on top, green below");

        let (half, w, h) = rotate_rgba(&pixels, 2, 1, Turns::none().right().right());
        assert_eq!((w, h), (2, 1));
        assert_eq!(half, [G, R].concat());

        let (left, w, h) = rotate_rgba(&pixels, 2, 1, Turns::none().left());
        assert_eq!((w, h), (1, 2));
        assert_eq!(left, [G, R].concat(), "green on top after a turn to the left");
    }

    #[test]
    fn four_quarter_turns_of_the_pixels_are_the_original() {
        let pixels: Vec<u8> = (0..(3 * 2 * 4)).map(|v| v as u8).collect();
        let mut current = (pixels.clone(), 3, 2);
        for _ in 0..4 {
            current = rotate_rgba(&current.0, current.1, current.2, Turns::none().right());
        }
        assert_eq!(current, (pixels, 3, 2));
    }
}
