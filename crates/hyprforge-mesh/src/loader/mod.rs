//! Which format a file is, and loading it into a [`Mesh`].
//!
//! Blocking, deliberately: view3d ran its own loader thread and a channel,
//! and the photo viewer already has a place for slow work (tokio's
//! blocking pool, where every decode goes). A library that spawned its
//! own threads would be a second scheduler nobody asked for.

pub mod obj;
pub mod stl;
pub mod threemf;

use anyhow::{bail, Result};
use std::path::Path;

use crate::mesh::Mesh;

/// Extensions this crate will open.
pub const EXTENSIONS: [&str; 3] = ["stl", "3mf", "obj"];

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Format {
    Stl,
    ThreeMf,
    Obj,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Stl, Format::ThreeMf, Format::Obj];

    /// What an inspector calls it.
    pub fn name(self) -> &'static str {
        match self {
            Format::Stl => "STL",
            Format::ThreeMf => "3MF",
            Format::Obj => "OBJ",
        }
    }

    /// The shared MIME database's canonical type — what a desktop entry
    /// claims so a file manager offers this app for the file. `model/stl`
    /// covers both of STL's encodings; the database lists
    /// `model/x.stl-binary` and `model/x.stl-ascii` as its aliases.
    pub fn mime_type(self) -> &'static str {
        match self {
            Format::Stl => "model/stl",
            Format::ThreeMf => "model/3mf",
            Format::Obj => "model/obj",
        }
    }
}

/// The format a path names, by extension. The extension is the claim
/// here, as it is for every loader in view3d: an STL has no reliable
/// magic (binary ones often begin with the word `solid`, which is what
/// ASCII ones begin with), and the loaders themselves report a file that
/// is not what its name says.
pub fn detect(path: &Path) -> Option<Format> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "stl" => Some(Format::Stl),
        "3mf" => Some(Format::ThreeMf),
        "obj" => Some(Format::Obj),
        _ => None,
    }
}

/// Loads `path`. The warning is for a file that loaded but not entirely
/// — an OBJ whose material library could not be read, drawn uncoloured.
///
/// `obj_y_up` rotates OBJ's conventional Y-up into the Z-up STL and 3MF
/// use, so the three formats sit the same way up on screen.
pub fn load(path: &Path, obj_y_up: bool) -> Result<(Mesh, Option<String>)> {
    match detect(path) {
        Some(Format::Stl) => Ok((stl::load(path)?, None)),
        Some(Format::ThreeMf) => Ok((threemf::load(path)?, None)),
        Some(Format::Obj) => obj::load(path, obj_y_up),
        None => bail!(
            "{} has an unsupported extension (expected .stl, .3mf or .obj)",
            path.display()
        ),
    }
}
