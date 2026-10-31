//! Reading a binary glTF (.glb): its triangles, placed as its nodes say, and each
//! triangle's colour (from the texture, the corners' colours or the material).

use serde_json::Value;

use super::Mesh;

/// A texture, as 8-bit RGB.
pub struct Texture {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Texture {
    /// The colour at (u, v), glTF's way round (v = 0 at the top), repeating outside 0..1.
    pub fn sample(&self, u: f64, v: f64) -> [u8; 3] {
        let x = ((u.rem_euclid(1.0) * self.width as f64) as usize).min(self.width - 1);
        let y = ((v.rem_euclid(1.0) * self.height as f64) as usize).min(self.height - 1);
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}

/// An embedded image's bytes.
fn image_bytes<'a>(image: &Value, views: &[Value], bin: &'a [u8]) -> Option<&'a [u8]> {
    let view = views.get(image.get("bufferView")?.as_u64()? as usize)?;
    let start = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let len = view.get("byteLength")?.as_u64()? as usize;
    bin.get(start..start + len)
}

/// A PNG texture (what ComfyUI saves); anything else isn't read (the model prints in one colour).
fn decode_png(bytes: &[u8]) -> Option<Texture> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let ch = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        _ => return None,
    };
    if w == 0 || h == 0 {
        return None;
    }
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let row = buf.get(y * info.line_size..)?;
        for x in 0..w {
            let p = row.get(x * ch..x * ch + ch)?;
            if ch >= 3 {
                rgb.extend_from_slice(&p[..3]);
            } else {
                rgb.extend_from_slice(&[p[0], p[0], p[0]]);
            }
        }
    }
    Some(Texture { width: w, height: h, rgb })
}

pub(super) fn bad(what: &str) -> String {
    format!("That 3D model couldn't be read ({what}).")
}

/// The triangles of a binary glTF (.glb), with each mesh's own placement applied.
pub fn read(data: &[u8]) -> Result<Mesh, String> {
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
    let materials = arr(&g, "materials");
    let textures = arr(&g, "textures");
    let images = arr(&g, "images");
    // Every embedded picture, decoded once (a model has one or two).
    let decoded: Vec<Option<Texture>> = images.iter().map(|i| image_bytes(i, &views, bin).and_then(decode_png)).collect();
    let mut coloured = false;

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

            // Each triangle's colour: from the texture, else the corners' colours, else the
            // material's own colour.
            let material = p.get("material").and_then(Value::as_u64).and_then(|i| materials.get(i as usize));
            let factor: Vec<f64> = material
                .and_then(|m| m.pointer("/pbrMetallicRoughness/baseColorFactor"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .filter(|v: &Vec<f64>| v.len() >= 3)
                .unwrap_or_else(|| vec![1.0, 1.0, 1.0]);
            let texture = material
                .and_then(|m| m.pointer("/pbrMetallicRoughness/baseColorTexture/index"))
                .and_then(Value::as_u64)
                .and_then(|t| textures.get(t as usize))
                .and_then(|t| t.get("source"))
                .and_then(Value::as_u64)
                .map(|img| img as usize);
            let texture = texture.and_then(|img| decoded.get(img)).and_then(Option::as_ref);
            let uv = p.pointer("/attributes/TEXCOORD_0").and_then(Value::as_u64).and_then(|a| accessors.get(a as usize));
            let corners = p.pointer("/attributes/COLOR_0").and_then(Value::as_u64).and_then(|a| accessors.get(a as usize));
            let tint = |c: [f64; 3]| -> [u8; 3] {
                [0, 1, 2].map(|i| (c[i] * factor[i] * 255.0).round().clamp(0.0, 255.0) as u8)
            };
            match (texture, uv, corners) {
                (Some(tex), Some(uv), _) => {
                    let (_, at) = read(uv)?;
                    for t in idx.chunks_exact(3) {
                        let uvs = [0, 1, 2].map(|k| (at(t[k] as usize, 0), at(t[k] as usize, 1)));
                        let mid = ((uvs[0].0 + uvs[1].0 + uvs[2].0) / 3.0, (uvs[0].1 + uvs[1].1 + uvs[2].1) / 3.0);
                        let mut sum = [0.0; 3];
                        for (u, v) in [uvs[0], uvs[1], uvs[2], mid, mid] {
                            let c = tex.sample(u, v);
                            for i in 0..3 {
                                sum[i] += c[i] as f64 / 255.0 / 5.0;
                            }
                        }
                        mesh.colors.push(tint(sum));
                    }
                    coloured = true;
                }
                (_, _, Some(acc)) => {
                    let scale = match acc.get("componentType").and_then(Value::as_u64) {
                        Some(5121) => 255.0,
                        Some(5123) => 65535.0,
                        _ => 1.0,
                    };
                    let (_, at) = read(acc)?;
                    for t in idx.chunks_exact(3) {
                        let mut sum = [0.0; 3];
                        for k in 0..3 {
                            for i in 0..3 {
                                sum[i] += at(t[k] as usize, i) / scale / 3.0;
                            }
                        }
                        mesh.colors.push(tint(sum));
                    }
                    coloured = true;
                }
                _ => {
                    for _ in idx.chunks_exact(3) {
                        mesh.colors.push(tint([1.0, 1.0, 1.0]));
                    }
                }
            }
        }
    }
    if mesh.triangles.is_empty() {
        return Err(bad("no triangles"));
    }
    // A plain shape: no colours to print.
    if !coloured {
        mesh.colors.clear();
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

