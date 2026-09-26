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
