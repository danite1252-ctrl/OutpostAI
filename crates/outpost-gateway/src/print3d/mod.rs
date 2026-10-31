//! 3D models ready for a slicer (Bambu Studio, PrusaSlicer, Cura...): a .3mf beside the
//! model's .glb, turned upright (Z up), standing on the bed, at a set height in millimetres,
//! with its corners joined so it's one closed solid. Options: a flat base or back (cut off
//! and closed, see [`cut`]), and up to four colours as filaments (see [`colours`]).

pub mod colours;
pub mod cut;
pub mod glb;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use outpost_engines::packs::Pack;
use serde_json::Value;

pub use glb::read as read_glb;
use glb::bad;

/// The height a model is printed at when nobody said.
pub const DEFAULT_HEIGHT_MM: f64 = 100.0;

/// The slicer copy sits beside the model's own file.
pub fn sidecar(model_file: &Path) -> PathBuf {
    // "v2": copies made before colours were written as colour groups are made again.
    model_file.with_extension("v2.3mf")
}

/// Its filament colours (`#rrggbb,...`), beside it too.
pub fn filaments_file(model_file: &Path) -> PathBuf {
    model_file.with_extension("filaments")
}

/// A triangle mesh, with each triangle's colour when it has them.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// One per triangle, or empty for a plain shape.
    pub colors: Vec<[u8; 3]>,
}

/// What it stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    AsMade,
    /// The bottom cut flat.
    FlatBottom,
    /// Laid on its back, cut flat (wall pieces, reliefs).
    FlatBack,
}

/// How the slicer copy is made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub height_mm: f64,
    /// Filaments: 1 to 4.
    pub colours: usize,
    pub base: Base,
}

impl Default for Options {
    fn default() -> Self {
        Self { height_mm: DEFAULT_HEIGHT_MM, colours: 1, base: Base::AsMade }
    }
}

impl Options {
    /// For a model made with "Ready for 3D printing" on; `None` when it's off.
    pub fn from_job(pack: &Pack, values: &serde_json::Map<String, Value>) -> Option<Self> {
        let height = pack.print_height(values)?;
        Some(Self::read(Some(pack), values, height))
    }

    /// The pack's own defaults (for a copy asked for later).
    pub fn defaults(pack: Option<&Pack>) -> Self {
        Self::read(pack, &serde_json::Map::new(), DEFAULT_HEIGHT_MM)
    }

    fn read(pack: Option<&Pack>, values: &serde_json::Map<String, Value>, height_mm: f64) -> Self {
        let choice = |key: &str| -> Option<String> {
            values.get(key).and_then(Value::as_str).map(str::to_string).or_else(|| {
                pack.and_then(|p| p.inputs.iter().find(|i| i.key == key)).and_then(|i| i.default.as_str().map(str::to_string))
            })
        };
        let colours = choice(outpost_engines::packs::PRINT_COLOURS)
            .and_then(|c| c.chars().find(|ch| ch.is_ascii_digit()).and_then(|d| d.to_digit(10)))
            .map(|d| (d as usize).clamp(1, 4))
            .unwrap_or(1);
        let base = match choice(outpost_engines::packs::PRINT_BASE).as_deref() {
            Some(b) if b.to_lowercase().contains("back") => Base::FlatBack,
            Some(b) if b.to_lowercase().contains("bottom") => Base::FlatBottom,
            _ => Base::AsMade,
        };
        Self { height_mm, colours, base }
    }
}

/// A model made ready to print: its mesh, and (for two or more colours) the filaments and
/// which one each triangle takes.
#[derive(Debug, Default)]
pub struct Printed {
    pub mesh: Mesh,
    pub palette: Vec<[u8; 3]>,
    pub paint: Vec<u8>,
}

fn bounds(v: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in v {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    (lo, hi)
}

/// Ready to print: glTF's Y-up turned to the slicer's Z-up (the model's front faces you),
/// scaled so it stands `height_mm` tall, centred, its lowest point on the bed, and its corners
/// joined (a .glb splits corners where its texture has seams, which would leave it open).
pub fn for_printing(mesh: &Mesh, o: &Options) -> Result<Printed, String> {
    let mut turned: Vec<[f64; 3]> = mesh.vertices.iter().map(|&[x, y, z]| [x, -z, y]).collect();
    let (lo, hi) = bounds(&turned);
    let tall = hi[2] - lo[2];
    if !tall.is_finite() || tall <= 0.0 {
        return Err(bad("it's flat"));
    }
    let k = o.height_mm / tall;
    if o.base == Base::FlatBack {
        // Laid on its back, face up.
        for p in turned.iter_mut() {
            *p = [p[0], p[2], -p[1]];
        }
    }
    let (lo, hi) = bounds(&turned);
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    // Corners closer than a ten-thousandth of a millimetre are the same corner.
    let key = |v: &[f64; 3]| ((v[0] * 1e4).round() as i64, (v[1] * 1e4).round() as i64, (v[2] * 1e4).round() as i64);
    let coloured = mesh.colors.len() == mesh.triangles.len() && !mesh.colors.is_empty();
    let mut out = Mesh::default();
    let mut seen: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut remap = Vec::with_capacity(turned.len());
    for v in &turned {
        let p = [(v[0] - mid[0]) * k, (v[1] - mid[1]) * k, (v[2] - lo[2]) * k];
        let i = *seen.entry(key(&p)).or_insert_with(|| {
            out.vertices.push(p);
            (out.vertices.len() - 1) as u32
        });
        remap.push(i);
    }
    let mut faces = std::collections::HashSet::new();
    for (ti, t) in mesh.triangles.iter().enumerate() {
        let t = [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]];
        if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
            continue; // Shrunk to a line or a point.
        }
        // The same triangle twice (a seam's two copies) is kept once.
        let mut sorted = t;
        sorted.sort_unstable();
        if faces.insert(sorted) {
            out.triangles.push(t);
            if coloured {
                out.colors.push(mesh.colors[ti]);
            }
        }
    }

    if o.base != Base::AsMade {
        // A thin slice off (3%, at least 0.3 mm), so it sits flat without losing much.
        let (lo, hi) = bounds(&out.vertices);
        let depth = ((hi[2] - lo[2]) * 0.03).max(0.3);
        let mut cut = out.clone();
        match cut::flatten(&mut cut, depth) {
            Ok(()) => out = cut,
            Err(e) => tracing::info!(reason = %e, "left the model's base as it was"),
        }
    }

    let mut printed = Printed { mesh: out, ..Default::default() };
    let m = &printed.mesh;
    if o.colours >= 2 && m.colors.len() == m.triangles.len() && !m.colors.is_empty() {
        let weights: Vec<f64> = m
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| m.vertices[i as usize]);
                let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0
            })
            .collect();
        let (palette, mut paint) = colours::reduce(&m.colors, &weights, o.colours);
        if palette.len() >= 2 {
            colours::tidy(&m.triangles, &mut paint, 3);
            printed.palette = palette;
            printed.paint = paint;
        }
    }
    Ok(printed)
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// A .3mf file (millimetres) holding one object.
pub fn write_3mf(printed: &Printed, title: &str) -> Result<Vec<u8>, String> {
    let mesh = &printed.mesh;
    let coloured = printed.palette.len() >= 2 && printed.paint.len() == mesh.triangles.len();
    let mut xml = String::with_capacity(mesh.vertices.len() * 60 + mesh.triangles.len() * 50 + 1024);
    let title: String = title
        .chars()
        .take(120)
        .map(|c| match c {
            '<' | '>' | '&' | '"' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(
        "<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" \
         xmlns:m=\"http://schemas.microsoft.com/3dmanufacturing/material/2015/02\">\n",
    );
    let _ = writeln!(xml, " <metadata name=\"Title\">{title}</metadata>");
    xml.push_str(" <metadata name=\"Application\">Outpost</metadata>\n");
    xml.push_str(" <resources>\n");
    // Colours: a 3MF colour group (the Materials extension), one colour per filament, most used
    // first; each triangle names its colour. Bambu Studio reads these from other apps' files
    // and maps them to filaments in this order (its own `paint_color` is only read from its
    // own projects).
    let object = if coloured {
        let names: Vec<String> = printed.palette.iter().map(|c| hex(*c)).collect();
        let _ = writeln!(xml, " <!-- Filaments: {} -->", names.join(", "));
        xml.push_str("  <m:colorgroup id=\"1\">\n");
        for c in &printed.palette {
            let _ = writeln!(xml, "   <m:color color=\"{}FF\"/>", hex(*c));
        }
        xml.push_str("  </m:colorgroup>\n");
        xml.push_str("  <object id=\"2\" type=\"model\" pid=\"1\" pindex=\"0\">\n");
        2
    } else {
        xml.push_str("  <object id=\"1\" type=\"model\">\n");
        1
    };
    xml.push_str("   <mesh>\n    <vertices>\n");
    let num = |n: f64| {
        let s = format!("{n:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".to_string() } else { s.to_string() }
    };
    for v in &mesh.vertices {
        let _ = writeln!(xml, "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", num(v[0]), num(v[1]), num(v[2]));
    }
    xml.push_str("    </vertices>\n    <triangles>\n");
    for (ti, t) in mesh.triangles.iter().enumerate() {
        if coloured {
            let f = printed.paint[ti];
            let _ = writeln!(xml, "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"1\" p1=\"{f}\"/>", t[0], t[1], t[2]);
        } else {
            let _ = writeln!(xml, "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>", t[0], t[1], t[2]);
        }
    }
    let _ = write!(
        xml,
        "    </triangles>\n   </mesh>\n  </object>\n </resources>\n <build>\n  <item objectid=\"{object}\"/>\n </build>\n</model>\n"
    );

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let files: [(&str, &str); 3] = [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/></Types>\n",
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/></Relationships>\n",
        ),
        ("3D/3dmodel.model", &xml),
    ];
    for (name, body) in files {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// A .glb made ready for printing: the .3mf, and its filament colours (empty for one).
pub fn convert(glb: &[u8], o: &Options, title: &str) -> Result<(Vec<u8>, Vec<[u8; 3]>), String> {
    let printed = for_printing(&read_glb(glb)?, o)?;
    Ok((write_3mf(&printed, title)?, printed.palette))
}

/// After a 3D model is saved: make its slicer copy if it was made "ready for 3D printing".
/// A failure only costs the copy (the app makes it again when asked).
pub async fn after_saved(state: &crate::AppState, pack: &Pack, values: &serde_json::Map<String, Value>, media_id: &str) {
    let Some(options) = Options::from_job(pack, values) else { return };
    let files = state.config.files_dir();
    let Ok((m, path)) = outpost_core::media::get_any(&state.pool, &files, media_id).await else { return };
    if m.kind != "model" {
        return;
    }
    let title = m.prompt.clone().unwrap_or_else(|| "Outpost 3D model".into());
    if let Err(e) = make(path, options, title).await {
        tracing::warn!(media = %media_id, error = %e, "couldn't make the 3D printing copy");
    }
}

/// Make the slicer copy of a saved model (in the background; slow for big models). Returns
/// its filament colours.
pub async fn make(model_file: PathBuf, o: Options, title: String) -> Result<Vec<[u8; 3]>, String> {
    tokio::task::spawn_blocking(move || {
        let glb = std::fs::read(&model_file).map_err(|e| e.to_string())?;
        let (out, palette) = convert(&glb, &o, &title)?;
        let dest = sidecar(&model_file);
        let part = dest.with_extension("part");
        std::fs::write(&part, out).map_err(|e| e.to_string())?;
        std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
        let names: Vec<String> = palette.iter().map(|c| hex(*c)).collect();
        let _ = std::fs::write(filaments_file(&model_file), names.join(","));
        Ok(palette)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// A glTF pyramid 2 units tall (Y up), its corners split like a textured model's.
    fn pyramid_glb() -> Vec<u8> {
        let corners: [[f32; 3]; 4] = [[-1., 0., -1.], [1., 0., -1.], [0., 0., 1.], [0., 2., 0.]];
        let tris: [[usize; 3]; 4] = [[0, 1, 2], [0, 3, 1], [1, 3, 2], [2, 3, 0]];
        let mut pos = Vec::new();
        let mut idx = Vec::new();
        for (i, t) in tris.iter().enumerate() {
            for (j, &c) in t.iter().enumerate() {
                for f in corners[c] {
                    pos.extend_from_slice(&f.to_le_bytes());
                }
                idx.extend_from_slice(&((i * 3 + j) as u32).to_le_bytes());
            }
        }
        let mut bin = pos.clone();
        bin.extend_from_slice(&idx);
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0, "translation": [5.0, 5.0, 5.0]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": pos.len()},
                {"buffer": 0, "byteOffset": pos.len(), "byteLength": idx.len()}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 12, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5125, "count": 12, "type": "SCALAR"}
            ]
        });
        let mut j = serde_json::to_vec(&json).unwrap();
        while j.len() % 4 != 0 {
            j.push(b' ');
        }
        let mut out = Vec::new();
        out.extend_from_slice(&0x4654_6C67u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + 8 + j.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(j.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        out.extend_from_slice(&j);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(&bin);
        out
    }

    #[test]
    fn a_glb_becomes_an_upright_closed_print_at_the_asked_height() {
        let mesh = read_glb(&pyramid_glb()).unwrap();
        assert!(mesh.colors.is_empty(), "a plain shape");
        assert_eq!((mesh.vertices.len(), mesh.triangles.len()), (12, 4));
        // The node's placement is applied.
        assert!(mesh.vertices.iter().all(|v| v[1] >= 5.0));
        let p = for_printing(&mesh, &Options::default()).unwrap().mesh;
        // The split corners are joined again: a closed pyramid has 4 corners.
        assert_eq!((p.vertices.len(), p.triangles.len()), (4, 4));
        let z: Vec<f64> = p.vertices.iter().map(|v| v[2]).collect();
        assert!((z.iter().cloned().fold(f64::INFINITY, f64::min)).abs() < 1e-9, "on the bed");
        assert!((z.iter().cloned().fold(0.0, f64::max) - 100.0).abs() < 1e-6, "100 mm tall");
        // Upright: the tip (glTF's up) is now the top.
        let tip = p.vertices.iter().find(|v| (v[2] - 100.0).abs() < 1e-6).unwrap();
        assert!(tip[0].abs() < 1e-6 && tip[1].abs() < 1e-6, "centred under the tip: {tip:?}");
        // Each edge is shared by exactly two triangles: one closed solid.
        let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &p.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 2));
    }

    #[test]
    fn the_3mf_is_a_package_slicers_open() {
        let (out, palette) = convert(&pyramid_glb(), &Options { height_mm: 50.0, ..Default::default() }, "A <little> pyramid").unwrap();
        assert!(palette.is_empty());
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(out)).unwrap();
        for name in ["[Content_Types].xml", "_rels/.rels"] {
            assert!(zip.by_name(name).is_ok(), "{name}");
        }
        let mut model = String::new();
        zip.by_name("3D/3dmodel.model").unwrap().read_to_string(&mut model).unwrap();
        assert!(model.contains("unit=\"millimeter\""));
        assert_eq!(model.matches("<vertex ").count(), 4);
        assert_eq!(model.matches("<triangle ").count(), 4);
        assert!(model.contains("z=\"50\""));
        assert!(model.contains("A  little  pyramid"));
    }

    #[test]
    fn not_a_model_is_refused() {
        assert!(read_glb(b"PK\x03\x04 not a glb").is_err());
        let mut cut = pyramid_glb();
        cut.truncate(cut.len() - 20);
        assert!(read_glb(&cut).is_err());
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use std::io::Read;

    fn model_xml(bytes: Vec<u8>) -> String {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut s = String::new();
        zip.by_name("3D/3dmodel.model").unwrap().read_to_string(&mut s).unwrap();
        s
    }

    /// A glTF-style (Y up) box 2 tall, front half red and back half blue.
    fn two_colour_box() -> Mesh {
        let mut m = cut::tests::tall_box();
        // The test box is Z up, 20 tall; make it glTF's Y up (the inverse of the turn).
        for p in m.vertices.iter_mut() {
            *p = [p[0] / 10.0, p[2] / 10.0, -p[1] / 10.0];
        }
        m.colors = (0..m.triangles.len()).map(|i| if i < 6 { [220, 20, 20] } else { [20, 20, 220] }).collect();
        m
    }

    #[test]
    fn colours_become_filaments_bambu_studio_reads() {
        let o = Options { colours: 4, ..Default::default() };
        let printed = for_printing(&two_colour_box(), &o).unwrap();
        assert_eq!(printed.palette.len(), 2, "only two colours there: {:?}", printed.palette);
        let xml = model_xml(write_3mf(&printed, "box").unwrap());
        assert!(xml.contains("xmlns:m=\"http://schemas.microsoft.com/3dmanufacturing/material/2015/02\""));
        assert!(xml.contains("<m:colorgroup id=\"1\">"));
        assert_eq!(xml.matches("<m:color color=").count(), 2);
        assert!(xml.contains("pid=\"1\" pindex=\"0\""));
        assert!(xml.contains("pid=\"1\" p1=\"1\""), "filament 2's triangles");
        assert!(xml.contains("pid=\"1\" p1=\"0\""), "filament 1's triangles");
        assert_eq!(xml.matches("<triangle ").count(), xml.matches(" p1=").count(), "every triangle says its colour");
        // One colour asked: a plain model.
        let plain = for_printing(&two_colour_box(), &Options::default()).unwrap();
        assert!(plain.palette.is_empty());
        assert!(!model_xml(write_3mf(&plain, "box").unwrap()).contains("colorgroup"));
    }

    #[test]
    fn a_flat_base_or_back_keeps_it_closed_and_on_the_bed() {
        for base in [Base::FlatBottom, Base::FlatBack] {
            let o = Options { base, colours: 2, ..Default::default() };
            let p = for_printing(&two_colour_box(), &o).unwrap();
            assert!(cut::tests::closed(&p.mesh), "{base:?}");
            let (lo, hi) = bounds(&p.mesh.vertices);
            assert_eq!(lo[2], 0.0);
            assert_eq!(p.paint.len(), p.mesh.triangles.len());
            if base == Base::FlatBack {
                // Lying down: 10 wide, 100 long (its height), about 50 high less the cut.
                assert!(hi[1] - lo[1] > 99.0 && hi[2] < 50.0, "{lo:?} {hi:?}");
            } else {
                assert!(hi[2] > 95.0 && hi[2] < 100.0, "a slice off the bottom: {hi:?}");
            }
        }
    }
}
