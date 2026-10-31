//! A flat base: everything below a level is cut off and the cut is closed with a flat face,
//! so the model stands on it.

use std::collections::{HashMap, HashSet};

use super::Mesh;

/// Cut the mesh `depth` above its lowest point, close the cut, and stand it on the cut (at
/// z = 0). Refuses (leaving the mesh as it was) when the cut would be ring-shaped (a cup's
/// rim), which a flat face can't simply close.
pub fn flatten(mesh: &mut Mesh, depth: f64) -> Result<(), String> {
    let lo = mesh.vertices.iter().map(|v| v[2]).fold(f64::INFINITY, f64::min);
    let hi = mesh.vertices.iter().map(|v| v[2]).fold(f64::NEG_INFINITY, f64::max);
    let zc = lo + depth;
    if !(zc < hi) {
        return Err("the cut would take the whole model".into());
    }
    let mut verts = mesh.vertices.clone();
    // Corners right at the cut are on it (no slivers).
    let eps = (hi - lo) * 1e-7;
    for v in verts.iter_mut() {
        if (v[2] - zc).abs() < eps {
            v[2] = zc;
        }
    }
    let coloured = mesh.colors.len() == mesh.triangles.len();
    let mut cuts: HashMap<(u32, u32), u32> = HashMap::new();
    let mut tris: Vec<[u32; 3]> = Vec::with_capacity(mesh.triangles.len());
    let mut colors: Vec<[u8; 3]> = Vec::new();
    for (ti, t) in mesh.triangles.iter().enumerate() {
        let below = t.map(|i| verts[i as usize][2] < zc);
        if !below.iter().any(|&b| b) {
            tris.push(*t);
            if coloured {
                colors.push(mesh.colors[ti]);
            }
            continue;
        }
        if below.iter().all(|&b| b) {
            continue;
        }
        // The part above the cut, keeping the triangle's winding.
        let mut poly: Vec<u32> = Vec::with_capacity(4);
        for e in 0..3 {
            let (c, n) = (t[e], t[(e + 1) % 3]);
            if !below[e] {
                poly.push(c);
            }
            if below[e] != below[(e + 1) % 3] {
                poly.push(crossing(c, n, zc, &mut verts, &mut cuts));
            }
        }
        poly.dedup();
        if poly.len() > 1 && poly.first() == poly.last() {
            poly.pop();
        }
        for i in 1..poly.len().saturating_sub(1) {
            let tri = [poly[0], poly[i], poly[i + 1]];
            if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
                tris.push(tri);
                if coloured {
                    colors.push(mesh.colors[ti]);
                }
            }
        }
    }

    // The open edges along the cut, and the face that closes them (its edges run the other way).
    let on = |i: u32, verts: &[[f64; 3]]| verts[i as usize][2] == zc;
    let edges: HashSet<(u32, u32)> = tris.iter().flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]).collect();
    let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut edge_colour: HashMap<[u8; 3], usize> = HashMap::new();
    for (ti, t) in tris.iter().enumerate() {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if on(a, &verts) && on(b, &verts) && !edges.contains(&(b, a)) {
                next.entry(b).or_default().push(a);
                if coloured {
                    *edge_colour.entry(colors[ti]).or_default() += 1;
                }
            }
        }
    }
    let mut loops: Vec<Vec<u32>> = Vec::new();
    while let Some(&start) = next.keys().next() {
        let mut lp = vec![start];
        let mut cur = start;
        let mut closed = false;
        loop {
            let Some(list) = next.get_mut(&cur) else { break };
            let n = list.pop();
            if list.is_empty() {
                next.remove(&cur);
            }
            let Some(n) = n else { break };
            if n == start {
                closed = true;
                break;
            }
            lp.push(n);
            cur = n;
            if lp.len() > verts.len() {
                break;
            }
        }
        if closed && lp.len() >= 3 {
            loops.push(lp);
        }
    }
    if loops.is_empty() {
        return Err("the cut left nothing to close".into());
    }
    // Seen from above, a face pointing down runs clockwise; one running the other way is a
    // hole in the cut (a ring), which this doesn't close.
    let area = |lp: &[u32]| -> f64 {
        (0..lp.len())
            .map(|i| {
                let (a, b) = (verts[lp[i] as usize], verts[lp[(i + 1) % lp.len()] as usize]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            / 2.0
    };
    if loops.iter().any(|lp| area(lp) > 0.0) {
        return Err("the cut is ring-shaped".into());
    }
    let cap_colour = edge_colour.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c);
    for lp in &loops {
        // Clockwise from above: clip it the other way round, then flip each triangle back.
        let ccw: Vec<u32> = lp.iter().rev().copied().collect();
        let pts: Vec<[f64; 2]> = ccw.iter().map(|&i| [verts[i as usize][0], verts[i as usize][1]]).collect();
        for [a, b, c] in ear_clip(&pts) {
            tris.push([ccw[a], ccw[c], ccw[b]]);
            if coloured {
                colors.push(cap_colour.unwrap_or([200, 200, 200]));
            }
        }
    }

    // Stand it on the cut, and keep only the corners still used.
    let mut used = vec![u32::MAX; verts.len()];
    let mut out = Mesh::default();
    for t in &tris {
        let mut nt = [0u32; 3];
        for k in 0..3 {
            let i = t[k] as usize;
            if used[i] == u32::MAX {
                used[i] = out.vertices.len() as u32;
                let v = verts[i];
                out.vertices.push([v[0], v[1], (v[2] - zc).max(0.0)]);
            }
            nt[k] = used[i];
        }
        out.triangles.push(nt);
    }
    out.colors = colors;
    *mesh = out;
    Ok(())
}

/// Where edge a–b (one corner below the cut, one not) meets the cut; shared by both
/// triangles of the edge, so the surface stays joined.
fn crossing(a: u32, b: u32, zc: f64, verts: &mut Vec<[f64; 3]>, cuts: &mut HashMap<(u32, u32), u32>) -> u32 {
    let (pa, pb) = (verts[a as usize], verts[b as usize]);
    if pa[2] == zc {
        return a;
    }
    if pb[2] == zc {
        return b;
    }
    let key = (a.min(b), a.max(b));
    if let Some(&i) = cuts.get(&key) {
        return i;
    }
    let t = (zc - pa[2]) / (pb[2] - pa[2]);
    verts.push([pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t, zc]);
    let i = (verts.len() - 1) as u32;
    cuts.insert(key, i);
    i
}

/// Triangles filling a simple polygon given anticlockwise (ear clipping), as indexes into it.
pub fn ear_clip(pts: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let inside = |p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        p != a && p != b && p != c && cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
    };
    let mut v: Vec<usize> = (0..pts.len()).collect();
    let mut out = Vec::with_capacity(pts.len().saturating_sub(2));
    let mut i = 0;
    let mut misses = 0;
    while v.len() > 3 {
        let m = v.len();
        let (p, c, n) = (v[(i + m - 1) % m], v[i % m], v[(i + 1) % m]);
        let convex = cross(pts[p], pts[c], pts[n]) > 0.0;
        let ear = convex && !v.iter().any(|&q| q != p && q != c && q != n && inside(pts[q], pts[p], pts[c], pts[n]));
        // No ear all the way round (rounding, or a corner on a straight line): take this one anyway.
        if ear || misses > m {
            if convex || misses > m {
                out.push([p, c, n]);
            }
            v.remove(i % m);
            misses = 0;
            i %= v.len().max(1);
        } else {
            i = (i + 1) % m;
            misses += 1;
        }
    }
    if v.len() == 3 {
        out.push([v[0], v[1], v[2]]);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A closed box, 10 wide and 20 tall, its triangles facing out.
    pub fn tall_box() -> Mesh {
        let v = |x: f64, y: f64, z: f64| [x, y, z];
        let vertices = vec![
            v(0., 0., 0.), v(10., 0., 0.), v(10., 10., 0.), v(0., 10., 0.),
            v(0., 0., 20.), v(10., 0., 20.), v(10., 10., 20.), v(0., 10., 20.),
        ];
        let triangles = vec![
            [0, 2, 1], [0, 3, 2], // bottom (down)
            [4, 5, 6], [4, 6, 7], // top (up)
            [0, 1, 5], [0, 5, 4], // front (-y)
            [1, 2, 6], [1, 6, 5], // right (+x)
            [2, 3, 7], [2, 7, 6], // back (+y)
            [3, 0, 4], [3, 4, 7], // left (-x)
        ];
        Mesh { vertices, triangles, colors: Vec::new() }
    }

    pub fn closed(m: &Mesh) -> bool {
        let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &m.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a, b)).or_default() += 1;
            }
        }
        edges.iter().all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1))
    }

    #[test]
    fn cutting_the_bottom_off_leaves_a_closed_solid_standing_on_the_cut() {
        let mut m = tall_box();
        // Tilt it so the cut goes through the sides, not along an existing edge.
        for p in m.vertices.iter_mut() {
            p[2] += p[0] * 0.1;
        }
        assert!(closed(&m));
        flatten(&mut m, 3.0).unwrap();
        assert!(closed(&m), "still one closed solid");
        let lo = m.vertices.iter().map(|v| v[2]).fold(f64::INFINITY, f64::min);
        assert_eq!(lo, 0.0);
        // The new base is flat: every corner at the bottom is at 0.
        let flat = m.vertices.iter().filter(|v| v[2].abs() < 1e-9).count();
        assert!(flat >= 3);
        // The base faces down.
        let down = m.triangles.iter().any(|t| {
            let [a, b, c] = t.map(|i| m.vertices[i as usize]);
            a[2] == 0.0 && b[2] == 0.0 && c[2] == 0.0 && {
                let n = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                n < 0.0
            }
        });
        assert!(down);
    }

    #[test]
    fn concave_cuts_are_filled_without_spilling() {
        // An L shape, anticlockwise.
        let pts = [[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 4.], [0., 4.]];
        let tris = ear_clip(&pts);
        assert_eq!(tris.len(), 4);
        let area: f64 = tris
            .iter()
            .map(|&[a, b, c]| {
                let (a, b, c) = (pts[a], pts[b], pts[c]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2.0
            })
            .sum();
        assert!((area - 7.0).abs() < 1e-9, "the L's area exactly: {area}");
    }
}
