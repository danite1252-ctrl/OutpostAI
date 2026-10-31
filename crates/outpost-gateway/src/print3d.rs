//! 3D models ready for a slicer (Bambu Studio, PrusaSlicer, Cura...): the model's shape as a
//! .3mf beside its .glb, turned upright (Z up), standing flat on the bed, at a set height in
//! millimetres, with its corners joined so it's one closed solid.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The height a model is printed at when nobody said.
pub const DEFAULT_HEIGHT_MM: f64 = 100.0;

/// The slicer copy sits beside the model's own file.
pub fn sidecar(model_file: &Path) -> PathBuf {
    model_file.with_extension("3mf")
}

/// A triangle mesh.
#[derive(Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

fn bad(what: &str) -> String {
    format!("That 3D model couldn't be read ({what}).")
}

/// The triangles of a binary glTF (.glb), with each mesh's own placement applied.
pub fn read_glb(data: &[u8]) -> Result<Mesh, String> {
    let u32_at = |i: usize| -> Option<u32> { data.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
    if u32_at(0) != Some(0x4654_6C67) || u32_at(4) != Some(2) {
        return Err(bad("not a .glb"));
    }
    let mut json: Option<Value> = None;
    let mut bin: &[u8] = &[];
    let mut at = 12;
    while let (Some(len), Some(kind)) = (u32_at(at), u32_at(at + 4)) {
        let start = at + 8;
        let chunk = data.get(start..start + len as usize).ok_or_else(|| bad("cut short"))?;
        match kind {
            0x4E4F_534A => json = Some(serde_json::from_slice(chunk).map_err(|_| bad("bad header"))?),
            0x004E_4942 => bin = chunk,
            _ => {}
        }
        at = start + len as usize;
    }
    let g = json.ok_or_else(|| bad("no header"))?;
    let arr = |v: &Value, k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let accessors = arr(&g, "accessors");
    let views = arr(&g, "bufferViews");
    let meshes = arr(&g, "meshes");

    // Which meshes to take, and where: the nodes that place them (or every mesh, unplaced).
    let mut placed: Vec<(usize, [f64; 16])> = arr(&g, "nodes")
        .iter()
        .filter_map(|n| Some((n.get("mesh")?.as_u64()? as usize, node_matrix(n))))
        .collect();
    if placed.is_empty() {
        placed = (0..meshes.len()).map(|i| (i, IDENTITY)).collect();
    }

    // The bytes of one accessor's element `i`, component `c` (of `comps`).
    let read = |acc: &Value| -> Result<(usize, Box<dyn Fn(usize, usize) -> f64 + '_>), String> {
        let count = acc.get("count").and_then(Value::as_u64).ok_or_else(|| bad("an accessor has no count"))? as usize;
        let ctype = acc.get("componentType").and_then(Value::as_u64).unwrap_or(0);
        let comps = match acc.get("type").and_then(Value::as_str) {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            _ => return Err(bad("an unknown accessor type")),
        };
        let size = match ctype {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(bad("an unknown number type")),
        };
        let view = acc
            .get("bufferView")
            .and_then(Value::as_u64)
            .and_then(|v| views.get(v as usize))
            .ok_or_else(|| bad("data outside the file"))?;
        let base = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize
            + acc.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let stride = view.get("byteStride").and_then(Value::as_u64).map(|s| s as usize).unwrap_or(size * comps);
        if count > 0 {
            let end = base + stride * (count - 1) + size * comps;
            if end > bin.len() {
                return Err(bad("data past the end"));
            }
        }
        Ok((
            count,
            Box::new(move |i: usize, c: usize| {
                let o = base + i * stride + c * size;
                let b = &bin[o..o + size];
                match ctype {
                    5126 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                    5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                    5123 => u16::from_le_bytes([b[0], b[1]]) as f64,
                    5122 => i16::from_le_bytes([b[0], b[1]]) as f64,
                    5121 => b[0] as f64,
                    _ => b[0] as i8 as f64,
                }
            }),
        ))
    };

    let mut mesh = Mesh::default();
    for (mi, m) in placed {
        let Some(prims) = meshes.get(mi).and_then(|m| m.get("primitives")).and_then(Value::as_array) else { continue };
        for p in prims {
            // Triangles only (mode 4, the default).
            if p.get("mode").and_then(Value::as_u64).unwrap_or(4) != 4 {
                continue;
            }
            let Some(pos) = p.pointer("/attributes/POSITION").and_then(Value::as_u64).and_then(|a| accessors.get(a as usize)) else {
                continue;
            };
            let (n, at) = read(pos)?;
            let first = mesh.vertices.len() as u32;
            for i in 0..n {
                mesh.vertices.push(apply(&m, [at(i, 0), at(i, 1), at(i, 2)]));
            }
            let idx: Vec<u32> = match p.get("indices").and_then(Value::as_u64).and_then(|a| accessors.get(a as usize)) {
                Some(acc) => {
                    let (k, at) = read(acc)?;
                    (0..k).map(|i| at(i, 0) as u32).collect()
                }
                None => (0..n as u32).collect(),
            };
            for t in idx.chunks_exact(3) {
                if t.iter().any(|&v| v as usize >= n) {
                    return Err(bad("a corner that isn't there"));
                }
                mesh.triangles.push([first + t[0], first + t[1], first + t[2]]);
            }
        }
    }
    if mesh.triangles.is_empty() {
        return Err(bad("no triangles"));
    }
    Ok(mesh)
}

const IDENTITY: [f64; 16] = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];

/// A node's own placement, column-major (glTF's `matrix`, or translation/rotation/scale).
fn node_matrix(n: &Value) -> [f64; 16] {
    let nums = |k: &str, len: usize| -> Option<Vec<f64>> {
        let v: Vec<f64> = n.get(k)?.as_array()?.iter().filter_map(Value::as_f64).collect();
        (v.len() == len).then_some(v)
    };
    if let Some(m) = nums("matrix", 16) {
        let mut out = [0.0; 16];
        out.copy_from_slice(&m);
        return out;
    }
    let t = nums("translation", 3).unwrap_or(vec![0., 0., 0.]);
    let r = nums("rotation", 4).unwrap_or(vec![0., 0., 0., 1.]);
    let s = nums("scale", 3).unwrap_or(vec![1., 1., 1.]);
    let (x, y, z, w) = (r[0], r[1], r[2], r[3]);
    // Rotation (from the quaternion) times scale, then the translation.
    let rot = [
        [1. - 2. * (y * y + z * z), 2. * (x * y - z * w), 2. * (x * z + y * w)],
        [2. * (x * y + z * w), 1. - 2. * (x * x + z * z), 2. * (y * z - x * w)],
        [2. * (x * z - y * w), 2. * (y * z + x * w), 1. - 2. * (x * x + y * y)],
    ];
    let mut out = IDENTITY;
    for col in 0..3 {
        for row in 0..3 {
            out[col * 4 + row] = rot[row][col] * s[col];
        }
    }
    out[12] = t[0];
    out[13] = t[1];
    out[14] = t[2];
    out
}

fn apply(m: &[f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

/// Ready to print: glTF's Y-up turned to the slicer's Z-up (the model's front faces you),
/// scaled to `height_mm`, centred, its lowest point on the bed, and its corners joined (a
/// .glb splits corners where its texture has seams, which would leave the solid open).
pub fn for_printing(mesh: &Mesh, height_mm: f64) -> Result<Mesh, String> {
    let turned: Vec<[f64; 3]> = mesh.vertices.iter().map(|&[x, y, z]| [x, -z, y]).collect();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for v in &turned {
        for a in 0..3 {
            lo[a] = lo[a].min(v[a]);
            hi[a] = hi[a].max(v[a]);
        }
    }
    let tall = hi[2] - lo[2];
    if !tall.is_finite() || tall <= 0.0 {
        return Err(bad("it's flat"));
    }
    let k = height_mm / tall;
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    // Corners closer than a ten-thousandth of a millimetre are the same corner.
    let key = |v: &[f64; 3]| ((v[0] * 1e4).round() as i64, (v[1] * 1e4).round() as i64, (v[2] * 1e4).round() as i64);
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
    for t in &mesh.triangles {
        let t = [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]];
        if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
            continue; // Shrunk to a line or a point.
        }
        // The same triangle twice (a seam's two copies) is kept once.
        let mut sorted = t;
        sorted.sort_unstable();
        if faces.insert(sorted) {
            out.triangles.push(t);
        }
    }
    Ok(out)
}

/// A .3mf file (millimetres) holding one object.
pub fn write_3mf(mesh: &Mesh, title: &str) -> Result<Vec<u8>, String> {
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
    xml.push_str("<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n");
    let _ = writeln!(xml, " <metadata name=\"Title\">{title}</metadata>");
    xml.push_str(" <metadata name=\"Application\">Outpost</metadata>\n");
    xml.push_str(" <resources>\n  <object id=\"1\" type=\"model\">\n   <mesh>\n    <vertices>\n");
    let num = |n: f64| {
        let s = format!("{n:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".to_string() } else { s.to_string() }
    };
    for v in &mesh.vertices {
        let _ = writeln!(xml, "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", num(v[0]), num(v[1]), num(v[2]));
    }
    xml.push_str("    </vertices>\n    <triangles>\n");
    for t in &mesh.triangles {
        let _ = writeln!(xml, "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>", t[0], t[1], t[2]);
    }
    xml.push_str("    </triangles>\n   </mesh>\n  </object>\n </resources>\n <build>\n  <item objectid=\"1\"/>\n </build>\n</model>\n");

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

/// A .glb made ready for printing, as .3mf bytes.
pub fn convert(glb: &[u8], height_mm: f64, title: &str) -> Result<Vec<u8>, String> {
    write_3mf(&for_printing(&read_glb(glb)?, height_mm)?, title)
}

/// After a 3D model is saved: make its slicer copy if it was made "ready for 3D printing".
/// A failure only costs the copy (the app makes it again when asked).
pub async fn after_saved(
    state: &crate::AppState,
    pack: &outpost_engines::packs::Pack,
    values: &serde_json::Map<String, Value>,
    media_id: &str,
) {
    let Some(height) = pack.print_height(values) else { return };
    let files = state.config.files_dir();
    let Ok((m, path)) = outpost_core::media::get_any(&state.pool, &files, media_id).await else { return };
    if m.kind != "model" {
        return;
    }
    let title = m.prompt.clone().unwrap_or_else(|| "Outpost 3D model".into());
    if let Err(e) = make(path, height, title).await {
        tracing::warn!(media = %media_id, error = %e, "couldn't make the 3D printing copy");
    }
}

/// Make the slicer copy of a saved model (in the background; slow for big models).
pub async fn make(model_file: PathBuf, height_mm: f64, title: String) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let glb = std::fs::read(&model_file).map_err(|e| e.to_string())?;
        let out = convert(&glb, height_mm, &title)?;
        let dest = sidecar(&model_file);
        let part = dest.with_extension("3mf.part");
        std::fs::write(&part, out).map_err(|e| e.to_string())?;
        std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
        Ok(dest)
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
        assert_eq!((mesh.vertices.len(), mesh.triangles.len()), (12, 4));
        // The node's placement is applied.
        assert!(mesh.vertices.iter().all(|v| v[1] >= 5.0));
        let p = for_printing(&mesh, 100.0).unwrap();
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
        let out = convert(&pyramid_glb(), 50.0, "A <little> pyramid").unwrap();
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
