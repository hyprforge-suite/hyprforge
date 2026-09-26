//! 3MF reading: OPC zip container + a streaming pull-parse of the 3D model
//! part, honouring build-item transforms, component trees, units, and colors.

use anyhow::{anyhow, bail, Context as _, Result};
use glam::{Mat4, Vec3};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use rustc_hash::FxHashMap;
use std::io::BufReader;
use std::path::Path;

use crate::mesh::{pack_rgb, weld, Mesh, WHITE};

const MAX_DEPTH: usize = 32;
const REL_TYPE_3DMODEL: &str = "3dmodel";

/// A reference to an object, possibly in another model part (the 3MF
/// production extension, which every slicer uses for project files).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct ObjectRef {
    part: String,
    id: u32,
}

#[derive(Default)]
struct Object {
    vertices: Vec<[f32; 3]>,
    triangles: Vec<Triangle>,
    components: Vec<(ObjectRef, Mat4)>,
    /// Default property group/index for triangles that do not carry their own.
    pid: Option<u32>,
    pindex: Option<u32>,
}

struct Triangle {
    v: [u32; 3],
    pid: Option<u32>,
    /// Per-corner property indices; corners 2 and 3 fall back to corner 1.
    p: [Option<u32>; 3],
}

/// One `*.model` part inside the container.
#[derive(Default)]
struct Part {
    objects: FxHashMap<u32, Object>,
    /// Property groups (basematerials, colorgroups) as flat color tables.
    palettes: FxHashMap<u32, Vec<u32>>,
    build: Vec<(ObjectRef, Mat4)>,
    unit_scale: f32,
}

/// Every model part in the container, keyed by its path inside the zip.
struct Model {
    parts: FxHashMap<String, Part>,
    root: String,
}

pub fn load(path: &Path) -> Result<Mesh> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("{} is not a valid 3MF (zip) container", path.display()))?;

    let root = model_part_name(&mut zip);
    let mut parts: FxHashMap<String, Part> = FxHashMap::default();
    let mut queue = vec![root.clone()];

    // Slicer project files split objects across parts via the production
    // extension, so follow every p:path reference until the graph is closed.
    while let Some(name) = queue.pop() {
        if parts.contains_key(&name) {
            continue;
        }
        let entry = zip
            .by_name(&name)
            .with_context(|| format!("3MF has no model part at {name}"))?;
        let part = parse_part(BufReader::new(entry), &name)?;

        for reference in part.build.iter().map(|(r, _)| r).chain(
            part.objects
                .values()
                .flat_map(|o| o.components.iter().map(|(r, _)| r)),
        ) {
            if reference.part != name && !parts.contains_key(&reference.part) {
                queue.push(reference.part.clone());
            }
        }
        parts.insert(name, part);
    }

    flatten(&Model { parts, root })
}

fn normalize_part(path: &str) -> String {
    path.trim_start_matches('/').to_owned()
}

/// Resolves the 3D model part from `_rels/.rels`, falling back to the
/// conventional path used by every writer in practice.
fn model_part_name<R: std::io::Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> String {
    const DEFAULT: &str = "3D/3dmodel.model";
    let Ok(rels) = zip.by_name("_rels/.rels") else {
        return DEFAULT.to_owned();
    };
    let mut reader = Reader::from_reader(BufReader::new(rels));
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == b"Relationship" {
                    let ty = attr(&e, b"Type").unwrap_or_default();
                    if ty.rsplit('/').next() == Some(REL_TYPE_3DMODEL) {
                        if let Some(target) = attr(&e, b"Target") {
                            return target.trim_start_matches('/').to_owned();
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    DEFAULT.to_owned()
}

fn attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.local_name().as_ref() == name)
            .then(|| {
                a.normalized_value(XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
            .flatten()
    })
}

fn attr_f32(e: &BytesStart<'_>, name: &[u8]) -> Option<f32> {
    attr(e, name)?.trim().parse().ok()
}

fn attr_u32(e: &BytesStart<'_>, name: &[u8]) -> Option<u32> {
    attr(e, name)?.trim().parse().ok()
}

/// 3MF transforms are 4x3, row-major, with translation in the last row.
fn parse_transform(s: &str) -> Option<Mat4> {
    let v: Vec<f32> = s
        .split_ascii_whitespace()
        .map(|t| t.parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    if v.len() != 12 {
        return None;
    }
    Some(Mat4::from_cols(
        glam::vec4(v[0], v[1], v[2], 0.0),
        glam::vec4(v[3], v[4], v[5], 0.0),
        glam::vec4(v[6], v[7], v[8], 0.0),
        glam::vec4(v[9], v[10], v[11], 1.0),
    ))
}

fn unit_scale(unit: &str) -> f32 {
    match unit {
        "micron" => 0.001,
        "centimeter" => 10.0,
        "inch" => 25.4,
        "foot" => 304.8,
        "meter" => 1000.0,
        // "millimeter" and anything unrecognised
        _ => 1.0,
    }
}

/// `#RRGGBB` or `#RRGGBBAA`.
fn parse_color(s: &str) -> Option<u32> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() < 6 {
        return None;
    }
    let c = |i: usize| {
        u8::from_str_radix(&h[i..i + 2], 16)
            .ok()
            .map(|v| v as f32 / 255.0)
    };
    Some(pack_rgb(c(0)?, c(2)?, c(4)?))
}

/// Parses one model part. `self_part` names it, so that references without a
/// `p:path` resolve inside this same part.
fn parse_part<R: std::io::BufRead>(reader: R, self_part: &str) -> Result<Part> {
    let mut xml = Reader::from_reader(reader);
    let mut buf = Vec::new();
    let mut model = Part {
        unit_scale: 1.0,
        ..Default::default()
    };

    // Parser state: which object / property group we are currently inside.
    let mut object: Option<(u32, Object)> = None;
    let mut palette: Option<(u32, Vec<u32>)> = None;

    loop {
        let event = xml.read_event_into(&mut buf)?;
        match event {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                match e.local_name().as_ref() {
                    b"model" => {
                        if let Some(u) = attr(e, b"unit") {
                            model.unit_scale = unit_scale(&u);
                        }
                    }
                    b"object" => {
                        let id =
                            attr_u32(e, b"id").ok_or_else(|| anyhow!("<object> without id"))?;
                        let obj = Object {
                            pid: attr_u32(e, b"pid"),
                            pindex: attr_u32(e, b"pindex"),
                            ..Default::default()
                        };
                        if empty {
                            model.objects.insert(id, obj);
                        } else {
                            object = Some((id, obj));
                        }
                    }
                    b"component" => {
                        if let (Some((_, obj)), Some(id)) =
                            (object.as_mut(), attr_u32(e, b"objectid"))
                        {
                            let m = attr(e, b"transform")
                                .and_then(|t| parse_transform(&t))
                                .unwrap_or(Mat4::IDENTITY);
                            let part = attr(e, b"path")
                                .map(|p| normalize_part(&p))
                                .unwrap_or_else(|| self_part.to_owned());
                            obj.components.push((ObjectRef { part, id }, m));
                        }
                    }
                    b"vertex" => {
                        if let Some((_, obj)) = object.as_mut() {
                            obj.vertices.push([
                                attr_f32(e, b"x").unwrap_or(0.0),
                                attr_f32(e, b"y").unwrap_or(0.0),
                                attr_f32(e, b"z").unwrap_or(0.0),
                            ]);
                        }
                    }
                    b"triangle" => {
                        if let Some((_, obj)) = object.as_mut() {
                            let (v1, v2, v3) =
                                (attr_u32(e, b"v1"), attr_u32(e, b"v2"), attr_u32(e, b"v3"));
                            if let (Some(a), Some(b), Some(c)) = (v1, v2, v3) {
                                obj.triangles.push(Triangle {
                                    v: [a, b, c],
                                    pid: attr_u32(e, b"pid"),
                                    p: [attr_u32(e, b"p1"), attr_u32(e, b"p2"), attr_u32(e, b"p3")],
                                });
                            }
                        }
                    }
                    b"basematerials" | b"colorgroup" => {
                        let id = attr_u32(e, b"id");
                        if let Some(id) = id {
                            if empty {
                                model.palettes.insert(id, Vec::new());
                            } else {
                                palette = Some((id, Vec::new()));
                            }
                        }
                    }
                    b"base" | b"color" => {
                        if let Some((_, colors)) = palette.as_mut() {
                            let raw = attr(e, b"displaycolor").or_else(|| attr(e, b"color"));
                            colors.push(raw.as_deref().and_then(parse_color).unwrap_or(WHITE));
                        }
                    }
                    b"item" => {
                        if let Some(id) = attr_u32(e, b"objectid") {
                            let m = attr(e, b"transform")
                                .and_then(|t| parse_transform(&t))
                                .unwrap_or(Mat4::IDENTITY);
                            let part = attr(e, b"path")
                                .map(|p| normalize_part(&p))
                                .unwrap_or_else(|| self_part.to_owned());
                            model.build.push((ObjectRef { part, id }, m));
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref e) => match e.local_name().as_ref() {
                b"object" => {
                    if let Some((id, obj)) = object.take() {
                        model.objects.insert(id, obj);
                    }
                }
                b"basematerials" | b"colorgroup" => {
                    if let Some((id, colors)) = palette.take() {
                        model.palettes.insert(id, colors);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }

    Ok(model)
}

/// Instantiates the build items (or, failing that, every object that nothing
/// else references) into a single welded mesh.
fn flatten(model: &Model) -> Result<Mesh> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<u32> = Vec::new();
    let mut has_colors = false;

    let unit_scale = model
        .parts
        .get(&model.root)
        .map(|p| p.unit_scale)
        .unwrap_or(1.0);
    let scale = Mat4::from_scale(Vec3::splat(unit_scale));

    let build: Vec<(ObjectRef, Mat4)> = model
        .parts
        .values()
        .flat_map(|p| p.build.iter().cloned())
        .collect();

    let roots: Vec<(ObjectRef, Mat4)> = if build.is_empty() {
        // No build section: draw every object nothing else references.
        let referenced: std::collections::HashSet<ObjectRef> = model
            .parts
            .values()
            .flat_map(|p| p.objects.values())
            .flat_map(|o| o.components.iter().map(|(r, _)| r.clone()))
            .collect();
        let mut refs: Vec<ObjectRef> = model
            .parts
            .iter()
            .flat_map(|(name, part)| {
                part.objects.keys().map(move |id| ObjectRef {
                    part: name.clone(),
                    id: *id,
                })
            })
            .filter(|r| !referenced.contains(r))
            .collect();
        refs.sort_by(|a, b| (&a.part, a.id).cmp(&(&b.part, b.id)));
        refs.into_iter().map(|r| (r, Mat4::IDENTITY)).collect()
    } else {
        build
    };

    for (reference, m) in roots {
        emit(
            model,
            &reference,
            scale * m,
            0,
            &mut positions,
            &mut colors,
            &mut has_colors,
        );
    }

    if positions.is_empty() {
        bail!("3MF model contains no triangles");
    }
    Ok(weld(positions, colors, has_colors))
}

fn emit(
    model: &Model,
    reference: &ObjectRef,
    xform: Mat4,
    depth: usize,
    positions: &mut Vec<[f32; 3]>,
    colors: &mut Vec<u32>,
    has_colors: &mut bool,
) {
    if depth > MAX_DEPTH {
        tracing::warn!("3MF component tree exceeds {MAX_DEPTH} levels; stopping recursion");
        return;
    }
    let Some(part) = model.parts.get(&reference.part) else {
        tracing::warn!("3MF references missing part {}", reference.part);
        return;
    };
    let Some(obj) = part.objects.get(&reference.id) else {
        tracing::warn!(
            "3MF references missing object {} in {}",
            reference.id,
            reference.part
        );
        return;
    };

    // Property groups live in the object's own part, but the root part is a
    // common place for shared materials, so fall back to it.
    let palette = |pid: u32| -> Option<&Vec<u32>> {
        part.palettes.get(&pid).or_else(|| {
            model
                .parts
                .get(&model.root)
                .and_then(|p| p.palettes.get(&pid))
        })
    };

    for tri in &obj.triangles {
        let pid = tri.pid.or(obj.pid);
        let corner_color = |corner: usize| -> u32 {
            let idx = tri.p[corner].or(tri.p[0]).or(obj.pindex);
            match (pid, idx) {
                (Some(pid), Some(idx)) => palette(pid)
                    .and_then(|p| p.get(idx as usize))
                    .copied()
                    .unwrap_or(WHITE),
                _ => WHITE,
            }
        };

        for (corner, &vi) in tri.v.iter().enumerate() {
            let Some(p) = obj.vertices.get(vi as usize) else {
                tracing::warn!("3MF object {} references missing vertex {vi}", reference.id);
                return;
            };
            let p = xform.transform_point3(Vec3::from(*p));
            positions.push(p.to_array());
            let c = corner_color(corner);
            if c != WHITE {
                *has_colors = true;
            }
            colors.push(c);
        }
    }

    for (child, m) in &obj.components {
        emit(
            model,
            child,
            xform * *m,
            depth + 1,
            positions,
            colors,
            has_colors,
        );
    }
}
