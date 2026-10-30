//! A clip from one humanoid played on another.
//!
//! Each mapped bone keeps the source bone's motion in the world: how far it
//! turned from its rest, applied to the target bone from its rest. Two
//! corrections make that hold between rigs that differ:
//!
//! - **Facing.** Each rig's own left, up and forward are read from its bones
//!   (the legs give left, the hips to the head give up), so a Mixamo export
//!   facing +Z drives a VRM 0 facing -Z the right way round.
//! - **Rest stance.** A T-pose clip on an A-pose body would lower the arms
//!   twice. Before the motion is applied, each target bone is turned from its
//!   rest direction onto the source bone's; the motion then starts from the
//!   same place on both.
//!
//! The hips also carry the clip's travel, scaled by the ratio of hip heights.
//! This is the approach of three.ws's `@three-ws/retarget` (Apache-2.0) and of
//! three-vrm's Mixamo example (MIT), written again here for any pair of rigs.

use crate::glb::Glb;
use crate::humanoid::{canonical_parent, HumanMap, BONES};
use crate::math::{M4, Q, V3};
use crate::scene::{Clip, Scene};
use serde_json::{json, Value};

pub struct Rig {
    pub scene: Scene,
    pub map: HumanMap,
    pub rest_m: Vec<M4>,
    pub rest_q: Vec<Q>,
    /// The rig's left, up and forward, as a rotation from +X, +Y, +Z.
    pub basis: Q,
    pub hips_height: f64,
}

impl Rig {
    pub fn read(g: &Glb) -> Result<Rig, String> {
        let scene = Scene::read(g)?;
        let map = HumanMap::read(g, &scene);
        let rest_m = scene.world(&scene.rest);
        let rest_q = scene.world_rot(&scene.rest);
        let mut rig = Rig { scene, map, rest_m, rest_q, basis: Q::ID, hips_height: 1.0 };
        rig.basis = rig.measure_basis();
        rig.hips_height = rig.measure_hips_height();
        Ok(rig)
    }

    pub fn pos(&self, bone: &str) -> Option<V3> {
        self.map.get(bone).map(|n| self.rest_m[n].origin())
    }

    fn up_hint(&self) -> Option<V3> {
        let hips = self.pos("hips")?;
        let top = self.pos("head").or(self.pos("neck")).or(self.pos("upperChest")).or(self.pos("chest")).or(self.pos("spine"))?;
        top.sub(hips).norm()
    }

    /// Up and left, each snapped to the nearest world axis. A body's own lines
    /// lean: the hips-to-head line of a mannequin that stands tall and of one
    /// that leans back differ by ten degrees, and that lean must stay a pose,
    /// not tilt the whole clip. Exports stand on an axis; glTF's up is +Y, and
    /// a file lying on its back (a Z-up export) still snaps to its own.
    fn measure_basis(&self) -> Q {
        let up = snap(self.up_hint().unwrap_or(V3(0.0, 1.0, 0.0)), None);
        let left = self
            .pos("leftUpperLeg")
            .zip(self.pos("rightUpperLeg"))
            .or(self.pos("leftUpperArm").zip(self.pos("rightUpperArm")))
            .map(|(l, r)| l.sub(r))
            .map(|l| snap(l, Some(up)))
            .unwrap_or_else(|| snap(V3(1.0, 0.0, 0.0), Some(up)));
        let fwd = left.cross(up);
        Q::from_basis(left, up, fwd)
    }

    fn measure_hips_height(&self) -> f64 {
        let Some(hips) = self.pos("hips") else { return 1.0 };
        let up = self.basis.rot(V3(0.0, 1.0, 0.0));
        let feet: Vec<f64> = ["leftFoot", "rightFoot", "leftToes", "rightToes"]
            .iter()
            .filter_map(|b| self.pos(b))
            .map(|p| p.dot(up))
            .collect();
        let floor = if feet.is_empty() {
            (0..self.scene.len()).map(|i| self.rest_m[i].origin().dot(up)).fold(f64::MAX, f64::min)
        } else {
            feet.iter().copied().fold(f64::MAX, f64::min)
        };
        let h = hips.dot(up) - floor;
        if h > 1e-6 { h } else { 1.0 }
    }

    /// The bones this rig and that one both have, in parent-first order.
    pub fn shared(&self, other: &Rig) -> Vec<&'static str> {
        BONES.iter().map(|(b, _)| *b).filter(|b| self.map.get(b).is_some() && other.map.get(b).is_some()).collect()
    }
}

/// The world axis (±X, ±Y, ±Z) nearest `v`, leaving out any along `not`.
fn snap(v: V3, not: Option<V3>) -> V3 {
    let axes = [
        V3(1.0, 0.0, 0.0),
        V3(-1.0, 0.0, 0.0),
        V3(0.0, 1.0, 0.0),
        V3(0.0, -1.0, 0.0),
        V3(0.0, 0.0, 1.0),
        V3(0.0, 0.0, -1.0),
    ];
    axes.into_iter()
        .filter(|a| not.is_none_or(|n| a.dot(n).abs() < 0.5))
        .max_by(|a, b| a.dot(v).partial_cmp(&b.dot(v)).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(V3(1.0, 0.0, 0.0))
}

pub struct Options {
    /// Keep the clip's height changes and drop its travel across the floor.
    pub in_place: bool,
}

/// One retargeted clip: key times, and per target node its rotations, plus
/// the hips' translations.
pub struct Baked {
    pub times: Vec<f64>,
    pub rotations: Vec<(usize, Vec<Q>)>,
    pub hips: Option<(usize, Vec<V3>)>,
    pub bones: usize,
}

pub fn bake(src: &Rig, clip: &Clip, dst: &Rig, opt: &Options) -> Result<Baked, String> {
    let shared = src.shared(dst);
    if !shared.contains(&"hips") {
        return Err("no hips found on both rigs".into());
    }
    let limbs = shared.iter().filter(|b| b.contains("Arm") || b.contains("Leg")).count();
    if shared.len() < 6 || limbs < 2 {
        return Err(format!(
            "only {} bones match between the two rigs ({}): a humanoid needs hips, a spine and limbs",
            shared.len(),
            shared.join(", ")
        ));
    }
    let align = dst.basis.mul(src.basis.inv());

    // Each target bone's turn onto the source's rest direction.
    let mut fix: Vec<(&str, Q)> = Vec::new();
    for b in &shared {
        let own = BONES.iter().find(|(x, _)| x == b).map(|(_, d)| *d).unwrap_or(&[]);
        let toward = own.iter().find(|c| src.map.get(c).is_some() && dst.map.get(c).is_some());
        // The hips and the head keep their own frame: both face forward at
        // rest on any rig, so the source's turn from rest is theirs as it is.
        // (The head once took the neck's correction, and a mannequin whose
        // neck leans forward at rest bowed every head it was put on.)
        let r = match (toward, *b == "hips" || *b == "head") {
            (_, true) => Q::ID,
            (Some(c), false) => {
                let ds = align.rot(src.pos(c).unwrap().sub(src.pos(b).unwrap()));
                let dt = dst.pos(c).unwrap().sub(dst.pos(b).unwrap());
                Q::arc(dt, ds)
            }
            // A bone with nothing beyond it turns with its parent.
            (None, false) => {
                let mut p = canonical_parent(b);
                loop {
                    match p {
                        Some(pb) => match fix.iter().find(|(x, _)| *x == pb) {
                            Some((_, q)) => break *q,
                            None => p = canonical_parent(pb),
                        },
                        None => break Q::ID,
                    }
                }
            }
        };
        fix.push((b, r));
    }

    let src_nodes: Vec<usize> = (0..src.scene.len()).collect();
    let times = clip.key_times(&src_nodes);
    let hips_s = src.map.get("hips").unwrap();
    let hips_t = dst.map.get("hips").unwrap();
    let k = dst.hips_height / src.hips_height;
    let up_t = dst.basis.rot(V3(0.0, 1.0, 0.0));
    let hips_parent_inv = match dst.scene.parent[hips_t] {
        Some(p) => dst.rest_m[p].inv().unwrap_or(M4::ID),
        None => M4::ID,
    };
    let rest_hips_s = src.rest_m[hips_s].origin();
    let rest_hips_t = dst.rest_m[hips_t].origin();

    let mapped_t: Vec<Option<usize>> = {
        let mut m = vec![None; dst.scene.len()];
        for (i, b) in shared.iter().enumerate() {
            m[dst.map.get(b).unwrap()] = Some(i);
        }
        m
    };
    let mut rotations: Vec<(usize, Vec<Q>)> = shared.iter().map(|b| (dst.map.get(b).unwrap(), Vec::with_capacity(times.len()))).collect();
    let mut hips_track = Vec::with_capacity(times.len());

    for &t in &times {
        let pose = clip.pose(&src.scene.rest, t);
        let ws = src.scene.world_rot(&pose);
        let wm = src.scene.world(&pose);
        // The target, walked parents first: a mapped bone takes its world
        // rotation from the source; anything else stays at rest under it.
        let mut wt = vec![Q::ID; dst.scene.len()];
        for &i in &dst.scene.order {
            let parent = dst.scene.parent[i].map(|p| wt[p]).unwrap_or(Q::ID);
            wt[i] = match mapped_t[i] {
                Some(si) => {
                    let b = shared[si];
                    let sn = src.map.get(b).unwrap();
                    let delta = ws[sn].mul(src.rest_q[sn].inv());
                    let delta = align.mul(delta).mul(align.inv());
                    let world = delta.mul(fix[si].1).mul(dst.rest_q[i]).norm();
                    let local = parent.inv().mul(world).norm();
                    let track = &mut rotations[si].1;
                    let local = match track.last() {
                        Some(prev) if prev.dot(local) < 0.0 => local.neg(),
                        _ => local,
                    };
                    track.push(local);
                    world
                }
                None => parent.mul(dst.scene.rest[i].r).norm(),
            };
        }
        let mut travel = align.rot(wm[hips_s].origin().sub(rest_hips_s)).scale(k);
        if opt.in_place {
            travel = up_t.scale(travel.dot(up_t));
        }
        hips_track.push(hips_parent_inv.point(rest_hips_t.add(travel)));
    }

    Ok(Baked { times, rotations, hips: Some((hips_t, hips_track)), bones: shared.len() })
}

/// Write a baked clip into a GLB as a new animation; returns its index.
pub fn write(g: &mut Glb, baked: &Baked, name: &str) -> usize {
    let times: Vec<f32> = baked.times.iter().map(|t| *t as f32).collect();
    let input = g.push_f32(&times, "SCALAR", true);
    let mut samplers = Vec::new();
    let mut channels = Vec::new();
    for (node, qs) in &baked.rotations {
        let flat: Vec<f32> = qs.iter().flat_map(|q| [q.0 as f32, q.1 as f32, q.2 as f32, q.3 as f32]).collect();
        let output = g.push_f32(&flat, "VEC4", false);
        channels.push(json!({ "sampler": samplers.len(), "target": { "node": node, "path": "rotation" } }));
        samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
    }
    if let Some((node, ps)) = &baked.hips {
        let flat: Vec<f32> = ps.iter().flat_map(|p| [p.0 as f32, p.1 as f32, p.2 as f32]).collect();
        let output = g.push_f32(&flat, "VEC3", false);
        channels.push(json!({ "sampler": samplers.len(), "target": { "node": node, "path": "translation" } }));
        samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
    }
    let taken: Vec<String> = g.list("animations").iter().filter_map(|a| a.get("name").and_then(Value::as_str).map(str::to_string)).collect();
    let mut unique = name.to_string();
    let mut n = 2;
    while taken.contains(&unique) {
        unique = format!("{name} {n}");
        n += 1;
    }
    let root = g.json.as_object_mut().expect("glTF JSON is an object");
    let anims = root.entry("animations").or_insert_with(|| json!([]));
    let anims = anims.as_array_mut().expect("animations is an array");
    anims.push(json!({ "name": unique, "samplers": samplers, "channels": channels }));
    anims.len() - 1
}
