//! Builds the small model files the loader tests read. Everything is written
//! under `target/` at test time so no binary blobs live in the repo.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Tests run in parallel and share the fixture directory, so every file is
/// written to a unique temporary and renamed into place.
fn write_atomic(path: &Path, bytes: &[u8]) {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp{}-{n}", std::process::id()));
    std::fs::write(&tmp, bytes).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

/// A 10 x 10 x 10 cube at the origin: 8 corners, 12 triangles.
pub const CUBE_TRIS: usize = 12;
pub const CUBE_VERTS: usize = 8;
pub const CUBE_SIZE: f32 = 10.0;

pub fn dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fixtures");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The cube's triangles as ((corner, corner, corner)) position triples.
pub fn cube_triangles() -> Vec<[[f32; 3]; 3]> {
    let s = CUBE_SIZE;
    let c = |i: usize| -> [f32; 3] {
        [
            if i & 1 != 0 { s } else { 0.0 },
            if i & 2 != 0 { s } else { 0.0 },
            if i & 4 != 0 { s } else { 0.0 },
        ]
    };
    // Two triangles per face, wound outwards.
    let faces = [
        [0, 2, 3, 1], // z = 0
        [4, 5, 7, 6], // z = s
        [0, 1, 5, 4], // y = 0
        [2, 6, 7, 3], // y = s
        [0, 4, 6, 2], // x = 0
        [1, 3, 7, 5], // x = s
    ];
    let mut out = Vec::new();
    for f in faces {
        out.push([c(f[0]), c(f[1]), c(f[2])]);
        out.push([c(f[0]), c(f[2]), c(f[3])]);
    }
    out
}

pub fn write_binary_stl() -> PathBuf {
    let path = dir().join("cube_binary.stl");
    let mut out = vec![0u8; 80];
    out.extend_from_slice(&(CUBE_TRIS as u32).to_le_bytes());
    for tri in cube_triangles() {
        out.extend_from_slice(&[0u8; 12]); // normal, ignored on read
        for v in tri {
            for c in v {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    write_atomic(&path, &out);
    path
}

pub fn write_ascii_stl() -> PathBuf {
    let path = dir().join("cube_ascii.stl");
    let mut s = String::from("solid cube\n");
    for tri in cube_triangles() {
        s.push_str("  facet normal 0 0 0\n    outer loop\n");
        for v in tri {
            s.push_str(&format!("      vertex {} {} {}\n", v[0], v[1], v[2]));
        }
        s.push_str("    endloop\n  endfacet\n");
    }
    s.push_str("endsolid cube\n");
    write_atomic(&path, s.as_bytes());
    path
}

pub fn write_obj() -> PathBuf {
    let d = dir();
    write_atomic(&d.join("cube.mtl"), b"newmtl red\nKd 1.0 0.0 0.0\n");

    let path = d.join("cube.obj");
    let mut s = String::from("mtllib cube.mtl\nusemtl red\n");
    let mut verts: Vec<[f32; 3]> = Vec::new();
    let mut faces: Vec<[usize; 3]> = Vec::new();
    for tri in cube_triangles() {
        let mut idx = [0usize; 3];
        for (i, v) in tri.iter().enumerate() {
            idx[i] = match verts.iter().position(|p| p == v) {
                Some(j) => j,
                None => {
                    verts.push(*v);
                    verts.len() - 1
                }
            };
        }
        faces.push(idx);
    }
    for v in &verts {
        s.push_str(&format!("v {} {} {}\n", v[0], v[1], v[2]));
    }
    for f in &faces {
        s.push_str(&format!("f {} {} {}\n", f[0] + 1, f[1] + 1, f[2] + 1));
    }
    write_atomic(&path, s.as_bytes());
    path
}

/// A 3MF exercising centimetre units, a build-item transform, a component
/// reference, and a color group.
pub fn write_3mf() -> PathBuf {
    let path = dir().join("cube.3mf");
    let mut verts: Vec<[f32; 3]> = Vec::new();
    let mut faces: Vec<[usize; 3]> = Vec::new();
    for tri in cube_triangles() {
        let mut idx = [0usize; 3];
        for (i, v) in tri.iter().enumerate() {
            idx[i] = match verts.iter().position(|p| p == v) {
                Some(j) => j,
                None => {
                    verts.push(*v);
                    verts.len() - 1
                }
            };
        }
        faces.push(idx);
    }

    let mut model = String::from(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<model unit="centimeter" xml:lang="en-US"
       xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"
       xmlns:m="http://schemas.microsoft.com/3dmanufacturing/material/2015/02">
 <resources>
  <m:colorgroup id="5">
   <m:color color="#FF0000FF"/>
   <m:color color="#00FF00FF"/>
  </m:colorgroup>
  <object id="1" type="model">
   <mesh>
    <vertices>
"##,
    );
    for v in &verts {
        model.push_str(&format!(
            "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n",
            v[0], v[1], v[2]
        ));
    }
    model.push_str("    </vertices>\n    <triangles>\n");
    for (i, f) in faces.iter().enumerate() {
        // Alternate between the two colors so both palette entries are used.
        model.push_str(&format!(
            "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"5\" p1=\"{}\"/>\n",
            f[0],
            f[1],
            f[2],
            i % 2
        ));
    }
    model.push_str(
        r##"    </triangles>
   </mesh>
  </object>
  <object id="2" type="model">
   <components>
    <component objectid="1" transform="1 0 0 0 1 0 0 0 1 10 0 0"/>
   </components>
  </object>
 </resources>
 <build>
  <item objectid="2"/>
 </build>
</model>
"##,
    );

    let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Id="rel0" Target="/3D/3dmodel.model" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>
"#;

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("_rels/.rels", opts).unwrap();
    zip.write_all(rels.as_bytes()).unwrap();
    zip.start_file("3D/3dmodel.model", opts).unwrap();
    zip.write_all(model.as_bytes()).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    write_atomic(&path, &bytes);
    path
}
