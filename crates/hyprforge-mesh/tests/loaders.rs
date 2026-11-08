mod fixtures;

use fixtures::{CUBE_SIZE, CUBE_TRIS, CUBE_VERTS};
use hyprforge_mesh::loader;
use hyprforge_mesh::mesh::Mesh;

fn load(path: &std::path::Path) -> Mesh {
    loader::load(path, true)
        .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()))
        .0
}

#[test]
fn binary_and_ascii_stl_agree() {
    let bin = load(&fixtures::write_binary_stl());
    let ascii = load(&fixtures::write_ascii_stl());

    for m in [&bin, &ascii] {
        assert_eq!(m.tri_count(), CUBE_TRIS);
        // Welding must collapse 36 loose corners down to the cube's 8.
        assert_eq!(m.verts.len(), CUBE_VERTS);
        assert_eq!(m.bounds.min.to_array(), [0.0; 3]);
        assert_eq!(m.bounds.max.to_array(), [CUBE_SIZE; 3]);
        assert!(!m.has_colors);
    }
    assert_eq!(bin.verts.len(), ascii.verts.len());
    assert_eq!(bin.indices.len(), ascii.indices.len());
}

#[test]
fn obj_reads_geometry_and_material_color() {
    let path = fixtures::write_obj();

    // Y-up correction off: the cube keeps its authored coordinates.
    let raw = loader::load(&path, false).unwrap().0;
    assert_eq!(raw.tri_count(), CUBE_TRIS);
    assert_eq!(raw.verts.len(), CUBE_VERTS);
    assert_eq!(raw.bounds.min.to_array(), [0.0; 3]);
    assert_eq!(raw.bounds.max.to_array(), [CUBE_SIZE; 3]);
    assert!(
        raw.has_colors,
        "diffuse color from the .mtl should be picked up"
    );
    // Kd 1 0 0 -> opaque red.
    assert_eq!(raw.verts[0].color, 0xff00_00ff);

    // Y-up correction on: +Y becomes +Z, so the box still spans the same
    // extents but with the Y range mirrored into negatives.
    let rotated = loader::load(&path, true).unwrap().0;
    assert_eq!(rotated.tri_count(), CUBE_TRIS);
    assert_eq!(rotated.bounds.min.to_array(), [0.0, -CUBE_SIZE, 0.0]);
    assert_eq!(rotated.bounds.max.to_array(), [CUBE_SIZE, 0.0, CUBE_SIZE]);
}

#[test]
fn threemf_applies_units_components_and_colors() {
    let m = load(&fixtures::write_3mf());

    assert_eq!(m.tri_count(), CUBE_TRIS);
    // Centimetre units scale the 10-unit cube to 100 mm, and the component
    // transform shifts it 10 units (=100 mm) along X.
    assert_eq!(m.bounds.min.to_array(), [100.0, 0.0, 0.0]);
    assert_eq!(m.bounds.max.to_array(), [200.0, 100.0, 100.0]);

    assert!(m.has_colors);
    let colors: std::collections::HashSet<u32> = m.verts.iter().map(|v| v.color).collect();
    assert!(colors.contains(&0xff00_00ff), "red from the color group");
    assert!(colors.contains(&0xff00_ff00), "green from the color group");
    // Two colors on shared corners means the weld must not merge them.
    assert!(m.verts.len() > CUBE_VERTS);
}

#[test]
fn all_three_formats_describe_the_same_box() {
    let stl = load(&fixtures::write_binary_stl());
    let obj = loader::load(&fixtures::write_obj(), false).unwrap().0;
    let threemf = load(&fixtures::write_3mf());

    assert_eq!(stl.tri_count(), obj.tri_count());
    assert_eq!(stl.tri_count(), threemf.tri_count());
    assert_eq!(stl.bounds.size(), obj.bounds.size());
    // The 3MF cube is in centimetres, so it is ten times larger.
    assert_eq!(threemf.bounds.size(), stl.bounds.size() * 10.0);
}

#[test]
fn unsupported_extension_is_rejected() {
    let path = fixtures::dir().join("model.step");
    std::fs::write(&path, b"not a mesh").unwrap();
    let err = loader::load(&path, true).unwrap_err().to_string();
    assert!(err.contains("unsupported extension"), "{err}");
}

/// A fresh directory of its own under the fixture root, so a test can
/// lay out a model beside files it must *not* reach.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = fixtures::dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const RED_MTL: &str = "newmtl red\nKd 1 0 0\n";
const RED_TRIANGLE: &str = "usemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

/// `mtllib` is a path the file chooses, and a file is not trusted to
/// choose one outside its own folder: `/dev/zero` read a line that never
/// ended until memory ran out, and a FIFO never answered at all — from
/// a thumbnailer, on browsing a folder. A model with a library somewhere
/// else is drawn uncoloured, with the warning a missing library gets.
#[test]
fn an_obj_cannot_name_a_material_library_outside_its_folder() {
    let root = scratch("mtl-escape");
    let model_dir = root.join("model");
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(root.join("outside.mtl"), RED_MTL).unwrap();
    let absolute = root.join("outside.mtl");

    for lib in ["../outside.mtl".to_owned(), absolute.display().to_string()] {
        let path = model_dir.join("escape.obj");
        std::fs::write(&path, format!("mtllib {lib}\n{RED_TRIANGLE}")).unwrap();
        let (mesh, warning) = loader::load(&path, false).unwrap();
        assert!(!mesh.has_colors, "{lib} was read");
        assert!(warning.is_some(), "{lib} refused without a word");
    }
}

/// The library beside the model is still read — the rule is about where,
/// not whether.
#[test]
fn an_obj_still_reads_the_material_library_beside_it() {
    let dir = scratch("mtl-beside");
    std::fs::write(dir.join("here.mtl"), RED_MTL).unwrap();
    let path = dir.join("here.obj");
    std::fs::write(&path, format!("mtllib here.mtl\n{RED_TRIANGLE}")).unwrap();
    let (mesh, warning) = loader::load(&path, false).unwrap();
    assert!(mesh.has_colors);
    assert!(warning.is_none());
}

/// Only a regular file of a material library's size is read: a directory,
/// a device or a pipe in its place is not a library, and a library of
/// many megabytes is not one either.
#[test]
fn an_obj_material_library_must_be_a_small_regular_file() {
    let dir = scratch("mtl-shape");
    std::fs::create_dir_all(dir.join("folder.mtl")).unwrap();
    let mut big = String::from(RED_MTL);
    while big.len() < 5 << 20 {
        big.push_str("# padding padding padding padding padding padding padding\n");
    }
    std::fs::write(dir.join("big.mtl"), big).unwrap();

    for lib in ["folder.mtl", "big.mtl"] {
        let path = dir.join("shape.obj");
        std::fs::write(&path, format!("mtllib {lib}\n{RED_TRIANGLE}")).unwrap();
        let (mesh, warning) = loader::load(&path, false).unwrap();
        assert!(!mesh.has_colors, "{lib} was read");
        assert!(warning.is_some(), "{lib} refused without a word");
    }
}

/// One line of an OBJ is read whole before it is parsed, so a file with
/// no newline in it is one allocation the size of the file.
#[test]
fn an_obj_line_longer_than_any_real_one_is_refused() {
    let dir = scratch("obj-long-line");
    let path = dir.join("long.obj");
    let mut text = String::from(RED_TRIANGLE);
    text.push('#');
    text.push_str(&"x".repeat(2 << 20));
    text.push('\n');
    std::fs::write(&path, text).unwrap();
    assert!(loader::load(&path, false).is_err());
}

/// A coordinate that is not a finite number is refused, in every format:
/// it makes the bounds, the camera and the thumbnail NaN.
#[test]
fn a_model_with_a_coordinate_that_is_not_finite_is_refused() {
    let dir = scratch("not-finite");
    let obj = dir.join("inf.obj");
    std::fs::write(&obj, "v 0 0 0\nv inf 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
    assert!(loader::load(&obj, false).is_err(), "OBJ");

    let ascii = dir.join("nan.stl");
    std::fs::write(
        &ascii,
        "solid x\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex NaN 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid x\n",
    )
    .unwrap();
    assert!(loader::load(&ascii, false).is_err(), "ASCII STL");

    let mut binary = vec![0u8; 80];
    binary.extend_from_slice(&1u32.to_le_bytes());
    binary.extend_from_slice(&[0u8; 12]);
    for p in [[0.0f32, 0.0, 0.0], [f32::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in p {
            binary.extend_from_slice(&c.to_le_bytes());
        }
    }
    binary.extend_from_slice(&[0u8; 2]);
    let bin = dir.join("inf.stl");
    std::fs::write(&bin, binary).unwrap();
    assert!(loader::load(&bin, false).is_err(), "binary STL");
}
