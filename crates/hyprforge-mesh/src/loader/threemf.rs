//! 3MF reading: OPC zip container + a streaming pull-parse of the 3D model
//! part, honouring build-item transforms, component trees, units, and colors.

use anyhow::{anyhow, bail, Context as _, Result};
use glam::{Mat4, Vec3};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use rustc_hash::FxHashMap;
use std::io::{BufReader, Read as _};
use std::path::Path;

use crate::mesh::{pack_rgb, weld, Mesh, WHITE};

const MAX_DEPTH: usize = 32;
const REL_TYPE_3DMODEL: &str = "3dmodel";

/// What one file may cost before it is refused. Files thumbnails every
/// .3mf in a folder it is shown, unasked, so a file has to be unable to
/// take the process down — and a 3MF is a zip of XML, both of which say
/// "this is small" on the outside and decide how big they are inside.
///
/// Each limit closes a different door. `part_bytes` bounds how much XML
/// is inflated and parsed (a deflate bomb is a few KB for a gigabyte);
/// `vertices` and `triangles` bound what the parse keeps; `positions`
/// and `instances` bound what the component tree multiplies that into,
/// which depth alone never did — a chain of objects each naming the next
/// twice is 2^32 copies at the depth cap, from a file of a few KB. The
/// numbers are far past any real model: a ten-million-triangle mesh is a
/// half-gigabyte STL, and a plate of copies is hundreds of instances.
struct Limits {
    /// Decompressed bytes of XML, across every model part of the file.
    part_bytes: u64,
    vertices: usize,
    triangles: usize,
    /// Triangle corners after instancing: what `weld` is handed.
    positions: usize,
    /// Objects instantiated by the component tree, empty ones included —
    /// a tree of nothing but references is still billions of calls.
    instances: usize,
}

const LIMITS: Limits = Limits {
    part_bytes: 1 << 30,
    vertices: 10_000_000,
    triangles: 10_000_000,
    positions: 50_000_000,
    instances: 1_000_000,
};

/// `_rels/.rels` names one part; a real one is a few hundred bytes.
const MAX_RELS_BYTES: u64 = 1 << 20;

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
    load_with(path, &LIMITS)
}

fn load_with(path: &Path, limits: &Limits) -> Result<Mesh> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let zip = zip::ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("{} is not a valid 3MF (zip) container", path.display()))?;
    load_zip(zip, limits)
}

fn load_zip<R: std::io::Read + std::io::Seek>(
    mut zip: zip::ZipArchive<R>,
    limits: &Limits,
) -> Result<Mesh> {

    let root = model_part_name(&mut zip);
    let mut parts: FxHashMap<String, Part> = FxHashMap::default();
    let mut queue = vec![root.clone()];
    let mut kept = Kept::default();
    let mut bytes_left = limits.part_bytes;

    // Slicer project files split objects across parts via the production
    // extension, so follow every p:path reference until the graph is closed.
    while let Some(name) = queue.pop() {
        if parts.contains_key(&name) {
            continue;
        }
        let mut entry = zip
            .by_name(&name)
            .with_context(|| format!("3MF has no model part at {name}"))?;
        // The declared size is the zip's claim and only a fast refusal; the
        // `take` is what holds, because a bomb is free to understate it.
        if entry.size() > bytes_left {
            bail!("3MF model part {name} is too large to open");
        }
        let mut bounded = (&mut entry).take(bytes_left + 1);
        let part = parse_part(BufReader::new(&mut bounded), &name, limits, &mut kept)?;
        // A part cut off by the limit would parse as a model that simply
        // ended early — drawn, wrong, and with no word said. One byte past
        // the budget is the tell that there was more.
        let read = bytes_left + 1 - bounded.limit();
        if read > bytes_left {
            bail!("3MF model part {name} is too large to open");
        }
        bytes_left -= read;

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

    flatten(&Model { parts, root }, limits)
}

/// What the parse has kept so far, across every part of one file.
#[derive(Default)]
struct Kept {
    vertices: usize,
    triangles: usize,
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
    let mut reader = Reader::from_reader(BufReader::new(rels.take(MAX_RELS_BYTES)));
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
    // The slicing below is by byte, and a multi-byte character would put
    // a cut inside it — a panic, from a colour attribute.
    if h.len() < 6 || !h.is_ascii() {
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
fn parse_part<R: std::io::BufRead>(
    reader: R,
    self_part: &str,
    limits: &Limits,
    kept: &mut Kept,
) -> Result<Part> {
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
                            kept.vertices += 1;
                            if kept.vertices > limits.vertices {
                                bail!("3MF has too many vertices to open");
                            }
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
                                kept.triangles += 1;
                                if kept.triangles > limits.triangles {
                                    bail!("3MF has too many triangles to open");
                                }
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
fn flatten(model: &Model, limits: &Limits) -> Result<Mesh> {
    let mut out = Sink::default();

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

    for (reference, m) in &roots {
        emit(model, reference, scale * *m, &mut Vec::new(), limits, &mut out)?;
    }

    if out.positions.is_empty() {
        bail!("3MF model contains no triangles");
    }
    super::ensure_finite(&out.positions)?;
    Ok(weld(out.positions, out.colors, out.has_colors))
}

/// What the component tree has produced so far.
#[derive(Default)]
struct Sink {
    positions: Vec<[f32; 3]>,
    colors: Vec<u32>,
    has_colors: bool,
    instances: usize,
}

/// Instantiates `reference` and everything under it. `path` is the chain
/// of references that led here, which is what tells a cycle from an
/// object merely used twice: a plate of two copies names one object from
/// two places, and only a reference to something already on the path
/// leads back into itself.
fn emit<'m>(
    model: &'m Model,
    reference: &'m ObjectRef,
    xform: Mat4,
    path: &mut Vec<&'m ObjectRef>,
    limits: &Limits,
    out: &mut Sink,
) -> Result<()> {
    if path.len() > MAX_DEPTH {
        tracing::warn!("3MF component tree exceeds {MAX_DEPTH} levels; stopping recursion");
        return Ok(());
    }
    if path.contains(&reference) {
        bail!(
            "3MF object {} in {} contains itself",
            reference.id,
            reference.part
        );
    }
    out.instances += 1;
    if out.instances > limits.instances {
        bail!("3MF component tree has too many instances to open");
    }
    let Some(part) = model.parts.get(&reference.part) else {
        tracing::warn!("3MF references missing part {}", reference.part);
        return Ok(());
    };
    let Some(obj) = part.objects.get(&reference.id) else {
        tracing::warn!(
            "3MF references missing object {} in {}",
            reference.id,
            reference.part
        );
        return Ok(());
    };

    // Checked before a single corner is pushed, so a refusal costs nothing
    // and the budget is the most that is ever allocated.
    if out.positions.len() + obj.triangles.len() * 3 > limits.positions {
        bail!("3MF component tree has too many triangles to open");
    }

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

        // All three corners, or none: a triangle abandoned after its first
        // corner leaves every later triangle in the list shifted by one.
        let corners = tri.v.map(|vi| obj.vertices.get(vi as usize));
        let [Some(a), Some(b), Some(c)] = corners else {
            tracing::warn!("3MF object {} references a missing vertex", reference.id);
            return Ok(());
        };
        for (corner, p) in [a, b, c].into_iter().enumerate() {
            let p = xform.transform_point3(Vec3::from(*p));
            out.positions.push(p.to_array());
            let c = corner_color(corner);
            if c != WHITE {
                out.has_colors = true;
            }
            out.colors.push(c);
        }
    }

    path.push(reference);
    for (child, m) in &obj.components {
        emit(model, child, xform * *m, path, limits, out)?;
    }
    path.pop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "3D/3dmodel.model";

    /// Loads one model part from its XML, the way `load` does after the zip.
    fn load_xml(xml: &str) -> Result<Mesh> {
        let part = parse_part(xml.as_bytes(), ROOT, &LIMITS, &mut Kept::default())?;
        let mut parts = FxHashMap::default();
        parts.insert(ROOT.to_owned(), part);
        flatten(&Model { parts, root: ROOT.to_owned() }, &LIMITS)
    }

    const TRIANGLE: &str = r#"<mesh><vertices>
        <vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/>
        </vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh>"#;

    /// An object that names itself as its own component is a loop, not a
    /// model: following it only stops at the depth cap, having drawn the
    /// same triangles thirty-three times — and two such references per
    /// object are four billion.
    #[test]
    fn a_component_that_contains_itself_is_refused() {
        let xml = format!(
            r#"<model><resources><object id="1">{TRIANGLE}<components><component objectid="1"/></components></object></resources>
            <build><item objectid="1"/></build></model>"#
        );
        let Err(err) = load_xml(&xml) else { panic!("a cyclic component tree loaded") };
        assert!(format!("{err:#}").contains("contains itself"), "{err:#}");
    }

    /// Depth alone is no bound: each level naming the next twice doubles
    /// the instances, so a few kilobytes of XML describe more triangles than
    /// memory holds. Twenty-one levels is two million leaves, and a
    /// thumbnailer that loads every .3mf in a folder must say no to it
    /// rather than spend a gigabyte finding out.
    #[test]
    fn a_component_tree_that_multiplies_past_the_budget_is_refused() {
        let levels = 21;
        let mut xml = format!(r#"<model><resources><object id="{levels}">{TRIANGLE}</object>"#);
        for id in (0..levels).rev() {
            let next = id + 1;
            xml.push_str(&format!(
                r#"<object id="{id}"><components><component objectid="{next}"/><component objectid="{next}"/></components></object>"#
            ));
        }
        xml.push_str(r#"</resources><build><item objectid="0"/></build></model>"#);
        let Err(err) = load_xml(&xml) else { panic!("an exponential component tree loaded") };
        assert!(format!("{err:#}").contains("too many"), "{err:#}");
    }

    /// The same object used twice is not a cycle — a plate of two copies
    /// is the commonest 3MF there is.
    #[test]
    fn an_object_used_twice_is_drawn_twice() {
        let xml = format!(
            r#"<model><resources><object id="2">{TRIANGLE}</object>
            <object id="1"><components><component objectid="2"/><component objectid="2" transform="1 0 0 0 1 0 0 0 1 5 0 0"/></components></object></resources>
            <build><item objectid="1"/></build></model>"#
        );
        let mesh = load_xml(&xml).unwrap();
        assert_eq!(mesh.tri_count(), 2);
    }

    /// A colour is hex digits; anything else is no colour, never a panic
    /// from slicing a multi-byte character in half.
    #[test]
    fn a_colour_that_is_not_ascii_is_no_colour() {
        assert_eq!(parse_color("#aéééé"), None);
        assert_eq!(parse_color("#ff0000"), Some(pack_rgb(1.0, 0.0, 0.0)));
    }

    /// A coordinate that is not a finite number makes the whole model's
    /// bounds infinite, and everything drawn from them — the camera, the
    /// thumbnail — NaN.
    #[test]
    fn a_vertex_that_is_not_finite_is_refused() {
        let xml = r#"<model><resources><object id="1"><mesh><vertices>
            <vertex x="0" y="0" z="0"/><vertex x="inf" y="0" z="0"/><vertex x="0" y="1" z="0"/>
            </vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources>
            <build><item objectid="1"/></build></model>"#;
        assert!(load_xml(xml).is_err());
    }

    fn zipped(model: &str) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
        use std::io::Write as _;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file(ROOT, options).unwrap();
        writer.write_all(model.as_bytes()).unwrap();
        zip::ZipArchive::new(writer.finish().unwrap()).unwrap()
    }

    fn one_triangle() -> String {
        format!(r#"<model><resources><object id="1">{TRIANGLE}</object></resources><build><item objectid="1"/></build></model>"#)
    }

    /// The model part is inflated and parsed only up to a budget: a few
    /// kilobytes of deflate are a gigabyte of XML.
    #[test]
    fn a_model_part_past_the_byte_budget_is_refused() {
        let xml = one_triangle();
        let fits = Limits { part_bytes: xml.len() as u64, ..LIMITS };
        assert!(load_zip(zipped(&xml), &fits).is_ok());
        let short = Limits { part_bytes: xml.len() as u64 - 1, ..LIMITS };
        let Err(err) = load_zip(zipped(&xml), &short) else { panic!("an oversized part loaded") };
        assert!(format!("{err:#}").contains("too large"), "{err:#}");
    }

    /// What the parse keeps is counted as it goes, so a part of nothing
    /// but vertices stops at the budget rather than at the end of memory.
    #[test]
    fn a_model_with_more_vertices_or_triangles_than_the_budget_is_refused() {
        let xml = one_triangle();
        let few_vertices = Limits { vertices: 2, ..LIMITS };
        assert!(load_zip(zipped(&xml), &few_vertices).is_err());
        let no_triangles = Limits { triangles: 0, ..LIMITS };
        assert!(load_zip(zipped(&xml), &no_triangles).is_err());
        let few_positions = Limits { positions: 2, ..LIMITS };
        assert!(load_zip(zipped(&xml), &few_positions).is_err());
    }
}
