//! A coloured model as a few filaments: its triangles' colours reduced to at most four (by
//! k-means, weighted by area), then tidied so stray single triangles take their neighbours'
//! colour (no speckles, fewer filament swaps).

use std::collections::HashMap;

/// The `k` colours that best stand for these (most used first), and which one each takes.
pub fn reduce(colors: &[[u8; 3]], weights: &[f64], k: usize) -> (Vec<[u8; 3]>, Vec<u8>) {
    // Group similar colours first (16 levels a channel): k-means then works on a few thousand.
    let mut bins: HashMap<u16, (f64, [f64; 3])> = HashMap::new();
    for (c, &w) in colors.iter().zip(weights) {
        let key = ((c[0] as u16 >> 4) << 8) | ((c[1] as u16 >> 4) << 4) | (c[2] as u16 >> 4);
        let e = bins.entry(key).or_insert((0.0, [0.0; 3]));
        e.0 += w;
        for i in 0..3 {
            e.1[i] += c[i] as f64 * w;
        }
    }
    let mut pts: Vec<([f64; 3], f64)> = bins
        .into_values()
        .filter(|(w, _)| *w > 0.0)
        .map(|(w, s)| ([s[0] / w, s[1] / w, s[2] / w], w))
        .collect();
    pts.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if pts.is_empty() {
        return (vec![[200, 200, 200]], vec![0; colors.len()]);
    }
    let k = k.clamp(1, 4).min(pts.len());
    let d2 = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>();

    // Start from the most used colour, then each time the one furthest from those chosen
    // (counting how much of the model it covers, so a few stray triangles don't get a filament).
    let mut centres = vec![pts[0].0];
    while centres.len() < k {
        let far = pts
            .iter()
            .map(|(p, w)| (centres.iter().map(|c| d2(*p, *c)).fold(f64::INFINITY, f64::min) * w.sqrt(), *p))
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        match far {
            Some((score, p)) if score > 0.0 => centres.push(p),
            _ => break,
        }
    }
    let nearest = |p: [f64; 3], centres: &[[f64; 3]]| -> usize {
        (0..centres.len())
            .min_by(|&a, &b| d2(p, centres[a]).partial_cmp(&d2(p, centres[b])).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0)
    };
    for _ in 0..20 {
        let mut sums = vec![([0.0; 3], 0.0); centres.len()];
        for (p, w) in &pts {
            let c = nearest(*p, &centres);
            for i in 0..3 {
                sums[c].0[i] += p[i] * w;
            }
            sums[c].1 += w;
        }
        let mut moved = 0.0;
        for (c, (s, w)) in centres.iter_mut().zip(&sums) {
            if *w > 0.0 {
                let n = [s[0] / w, s[1] / w, s[2] / w];
                moved += d2(*c, n);
                *c = n;
            }
        }
        if moved < 0.01 {
            break;
        }
    }

    // Each triangle's colour, then the colours by how much they cover (filament 1 the most).
    let f = |c: [u8; 3]| [c[0] as f64, c[1] as f64, c[2] as f64];
    let raw: Vec<usize> = colors.iter().map(|&c| nearest(f(c), &centres)).collect();
    let mut cover = vec![0.0; centres.len()];
    for (&i, &w) in raw.iter().zip(weights) {
        cover[i] += w;
    }
    let mut order: Vec<usize> = (0..centres.len()).filter(|&i| cover[i] > 0.0).collect();
    order.sort_by(|&a, &b| cover[b].partial_cmp(&cover[a]).unwrap_or(std::cmp::Ordering::Equal));
    let mut rank = vec![0u8; centres.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r as u8;
    }
    let palette = order.iter().map(|&i| centres[i].map(|v| v.round().clamp(0.0, 255.0) as u8)).collect();
    (palette, raw.into_iter().map(|i| rank[i]).collect())
}

/// Triangles whose neighbours (at least two of their three) share a different colour take it.
pub fn tidy(triangles: &[[u32; 3]], paint: &mut [u8], passes: usize) {
    let mut by_edge: HashMap<(u32, u32), Vec<u32>> = HashMap::with_capacity(triangles.len() * 2);
    for (ti, t) in triangles.iter().enumerate() {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            by_edge.entry((a.min(b), a.max(b))).or_default().push(ti as u32);
        }
    }
    for _ in 0..passes {
        let before = paint.to_vec();
        let mut changed = false;
        for (ti, t) in triangles.iter().enumerate() {
            let mut counts = [0u8; 4];
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                if let Some(list) = by_edge.get(&(a.min(b), a.max(b))) {
                    for &o in list {
                        if o as usize != ti {
                            counts[(before[o as usize] as usize).min(3)] += 1;
                        }
                    }
                }
            }
            let (best, n) = counts.iter().enumerate().max_by_key(|(_, n)| **n).map(|(c, n)| (c as u8, *n)).unwrap_or((0, 0));
            if n >= 2 && best != before[ti] && counts[before[ti] as usize] == 0 {
                paint[ti] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_reduce_to_the_main_ones_most_used_first() {
        let mut colors = Vec::new();
        // Mostly red, some blue, a little near-red.
        colors.extend(std::iter::repeat([200, 30, 30]).take(60));
        colors.extend(std::iter::repeat([30, 40, 210]).take(30));
        colors.extend(std::iter::repeat([210, 40, 35]).take(10));
        let weights = vec![1.0; colors.len()];
        let (palette, paint) = reduce(&colors, &weights, 2);
        assert_eq!(palette.len(), 2);
        assert!(palette[0][0] > 150 && palette[0][2] < 80, "red first: {palette:?}");
        assert!(palette[1][2] > 150, "then blue");
        assert_eq!(paint[0], 0);
        assert_eq!(paint[70], 1);
        assert_eq!(paint[95], 0, "near-red joins red");
        // One colour asked: everything is filament 1.
        let (one, paint) = reduce(&colors, &weights, 1);
        assert_eq!(one.len(), 1);
        assert!(paint.iter().all(|&p| p == 0));
    }

    #[test]
    fn a_stray_triangle_takes_its_neighbours_colour() {
        // A fan of 6 triangles round a centre; one odd one out.
        let triangles: Vec<[u32; 3]> = (0..6u32).map(|i| [0, 1 + i, 1 + (i + 1) % 6]).collect();
        let mut paint = vec![0, 0, 1, 0, 0, 0];
        tidy(&triangles, &mut paint, 2);
        assert_eq!(paint, vec![0; 6]);
    }
}
