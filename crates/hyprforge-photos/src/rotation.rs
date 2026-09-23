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
}
