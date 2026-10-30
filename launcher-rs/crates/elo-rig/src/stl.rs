//! A model as a block GitHub draws in a README: an ASCII STL in a ```` ```stl ````
//! fence, which github.com renders as a 3D viewer you can drag.
//!
//! The triangles are the model at rest (a skinned mesh is posed through its
//! skin, so an A-pose stays an A-pose), turned Z-up, stood on the floor,
//! scaled to 100 units tall and thinned by vertex clustering until they fit
//! the budget. GitHub stops rendering a file past 512 KiB; ~1,500 triangles
//! is about 200 KiB. The idea is three.ws's `readme-3d` (Apache-2.0).

use crate::glb::Glb;
use crate::math::{M4, V3};
use crate::scene::Scene;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write;

pub const MAX_TRIANGLES: usize = 3000;

pub fn readme_stl(bytes: &[u8], triangles: usize, name: &str) -> Result<String, String> {
    let g = Glb::parse(bytes)?;
    let soup = triangles_at_rest(&g)?;
    if soup.is_empty() {
        return Err("the file has no triangles to draw".into());
    }
    let soup = stand_up(soup);
    let target = triangles.clamp(100, MAX_TRIANGLES);
    let mut tris = thin(&soup, target);
    // Merging corners averages them, which lifts the feet: stand it back down.
    let floor = tris.iter().flatten().map(|v| v.2).fold(f64::MAX, f64::min);
    for v in tris.iter_mut().flatten() {
        v.2 -= floor;
    }
    let solid: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect();
    let solid = if solid.is_empty() { "model".to_string() } else { solid };
    let mut out = format!("```stl\nsolid {solid}\n");
    for t in &tris {
        let n = t[1].sub(t[0]).cross(t[2].sub(t[0])).norm().unwrap_or(V3(0.0, 0.0, 1.0));
        let _ = writeln!(out, " facet normal {:.2} {:.2} {:.2}\n  outer loop", n.0, n.1, n.2);
        for v in t {
            let _ = writeln!(out, "   vertex {:.2} {:.2} {:.2}", v.0, v.1, v.2);
        }
        out.push_str("  endloop\n endfacet\n");
    }
    let _ = write!(out, "endsolid {solid}\n```\n");
    Ok(out)
}

fn triangles_at_rest(g: &Glb) -> Result<Vec<[V3; 3]>, String> {
    let scene = Scene::read(g)?;
    let world = scene.world(&scene.rest);
    let mut soup = Vec::new();
    for (ni, node) in g.list("nodes").iter().enumerate() {
        let Some(mesh) = node.get("mesh").and_then(Value::as_u64) else { continue };
        let Some(mesh) = g.list("meshes").get(mesh as usize) else { continue };
        let skin = node.get("skin").and_then(Value::as_u64).and_then(|s| g.list("skins").get(s as usize));
        let joint_m: Option<Vec<M4>> = match skin {
            Some(skin) => {
                let joints: Vec<usize> =
                    skin["joints"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|j| j as usize).collect();
                let ibm = match skin.get("inverseBindMatrices").and_then(Value::as_u64) {
                    Some(a) => g.read(a as usize)?.0,
                    None => Vec::new(),
                };
                Some(
                    joints
                        .iter()
                        .enumerate()
                        .map(|(k, j)| {
                            let inv = ibm.get(k * 16..k * 16 + 16).map(|m| {
                                let mut a = [0.0; 16];
                                a.copy_from_slice(m);
                                M4(a)
                            });
                            world.get(*j).copied().unwrap_or(M4::ID).mul(&inv.unwrap_or(M4::ID))
                        })
                        .collect(),
                )
            }
            None => None,
        };
        for prim in mesh["primitives"].as_array().into_iter().flatten() {
            if prim.pointer("/extensions/KHR_draco_mesh_compression").is_some() {
                return Err("this model is Draco-compressed; export it without mesh compression".into());
            }
            if prim.get("mode").and_then(Value::as_u64).unwrap_or(4) != 4 {
                continue;
            }
            let Some(pa) = prim.pointer("/attributes/POSITION").and_then(Value::as_u64) else { continue };
            let (pos, _) = g.read(pa as usize)?;
            let count = pos.len() / 3;
            let mut verts: Vec<V3> = (0..count).map(|i| V3::from(&pos[i * 3..i * 3 + 3])).collect();
            let skinned = match (&joint_m, prim.pointer("/attributes/JOINTS_0"), prim.pointer("/attributes/WEIGHTS_0")) {
                (Some(jm), Some(j), Some(w)) => {
                    let (js, _) = g.read(j.as_u64().unwrap_or(u64::MAX) as usize)?;
                    let (ws, _) = g.read(w.as_u64().unwrap_or(u64::MAX) as usize)?;
                    for (i, v) in verts.iter_mut().enumerate() {
                        let mut acc = V3::ZERO;
                        let mut total = 0.0;
                        for k in 0..4 {
                            let (Some(j), Some(w)) = (js.get(i * 4 + k), ws.get(i * 4 + k)) else { continue };
                            if *w <= 0.0 {
                                continue;
                            }
                            if let Some(m) = jm.get(*j as usize) {
                                acc = acc.add(m.point(*v).scale(*w));
                                total += w;
                            }
                        }
                        if total > 1e-6 {
                            *v = acc.scale(1.0 / total);
                        }
                    }
                    true
                }
                _ => false,
            };
            if !skinned {
                for v in verts.iter_mut() {
                    *v = world[ni].point(*v);
                }
            }
            let idx: Vec<usize> = match prim.get("indices").and_then(Value::as_u64) {
                Some(a) => g.read(a as usize)?.0.iter().map(|x| *x as usize).collect(),
                None => (0..count).collect(),
            };
            for t in idx.chunks_exact(3) {
                if let (Some(a), Some(b), Some(c)) = (verts.get(t[0]), verts.get(t[1]), verts.get(t[2])) {
                    soup.push([*a, *b, *c]);
                }
            }
        }
    }
    Ok(soup)
}

/// Y-up to Z-up, centred, on the floor, 100 units tall.
fn stand_up(soup: Vec<[V3; 3]>) -> Vec<[V3; 3]> {
    let (mut lo, mut hi) = (V3(f64::MAX, f64::MAX, f64::MAX), V3(f64::MIN, f64::MIN, f64::MIN));
    for v in soup.iter().flatten() {
        lo = V3(lo.0.min(v.0), lo.1.min(v.1), lo.2.min(v.2));
        hi = V3(hi.0.max(v.0), hi.1.max(v.1), hi.2.max(v.2));
    }
    let tall = (hi.1 - lo.1).max(hi.0 - lo.0).max(hi.2 - lo.2).max(1e-9);
    let k = 100.0 / tall;
    let c = V3((lo.0 + hi.0) / 2.0, lo.1, (lo.2 + hi.2) / 2.0);
    soup.into_iter()
        .map(|t| t.map(|v| {
            let p = v.sub(c).scale(k);
            V3(p.0, -p.2, p.1)
        }))
        .collect()
}

/// Snap every corner to a grid, merge what lands in one cell, drop what
/// collapses; widen the grid until the count fits.
fn cluster(soup: &[[V3; 3]], cell: f64) -> Vec<[V3; 3]> {
    let key = |v: V3| ((v.0 / cell).floor() as i64, (v.1 / cell).floor() as i64, (v.2 / cell).floor() as i64);
    let mut sum: HashMap<(i64, i64, i64), (V3, f64)> = HashMap::new();
    for v in soup.iter().flatten() {
        let e = sum.entry(key(*v)).or_insert((V3::ZERO, 0.0));
        e.0 = e.0.add(*v);
        e.1 += 1.0;
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for t in soup {
        let k = [key(t[0]), key(t[1]), key(t[2])];
        if k[0] == k[1] || k[1] == k[2] || k[0] == k[2] {
            continue;
        }
        let mut sorted = k;
        sorted.sort();
        if !seen.insert(sorted) {
            continue;
        }
        out.push(k.map(|c| {
            let (s, n) = sum[&c];
            s.scale(1.0 / n)
        }));
    }
    out
}

fn thin(soup: &[[V3; 3]], target: usize) -> Vec<[V3; 3]> {
    if soup.len() <= target {
        return soup.to_vec();
    }
    // The count falls as the cell grows: search the cell size in log space.
    let (mut lo, mut hi) = (0.01_f64, 50.0_f64);
    let mut best = cluster(soup, hi);
    for _ in 0..24 {
        let mid = (lo * hi).sqrt();
        let got = cluster(soup, mid);
        if got.len() <= target {
            hi = mid;
            best = got;
        } else {
            lo = mid;
        }
        if hi / lo < 1.02 {
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinning_meets_the_budget() {
        // A 60×60 grid of quads: 7,200 triangles.
        let mut soup = Vec::new();
        for a in 0..60 {
            for b in 0..60 {
                let p = |a: i32, b: i32| V3(a as f64, (a as f64 * 0.3).sin() * 4.0, b as f64);
                soup.push([p(a, b), p(a + 1, b), p(a + 1, b + 1)]);
                soup.push([p(a, b), p(a + 1, b + 1), p(a, b + 1)]);
            }
        }
        let thin = thin(&stand_up(soup), 1000);
        assert!(thin.len() <= 1000 && thin.len() > 500, "{}", thin.len());
    }
}
