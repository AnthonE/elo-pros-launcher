//! The rig desk: the files modders and VTubers already pass around, read and
//! written in the formats their tools already take.
//!
//! - [`inspect`]: which bone is which (VRM 1.0 humanoid names) and what clips a
//!   GLB, VRM or VRMA carries.
//! - [`retarget`]: copy clips from one humanoid onto another — Mixamo,
//!   Quaternius and Mesh2Motion (Unreal names), Rigify and Meshy, VRoid, VRM,
//!   Character Creator, Daz — and get the target back with the clips in it.
//! - [`vrma`]: a clip as a VRM Animation (`.vrma`), which three-vrm, UniVRM,
//!   the Blender VRM add-on and VRoid Hub play on any VRM.
//! - [`readme_stl`]: a model as a ```` ```stl ```` block GitHub draws as a
//!   live 3D viewer in a README.
//!
//! Everything runs on the caller's machine — natively, or in the browser as
//! wasm32 (`wasm.rs`). Nothing is uploaded.

mod glb;
mod humanoid;
mod math;
mod retarget;
mod scene;
mod stl;
mod vrma;
#[cfg(test)]
mod checks;
#[cfg(target_arch = "wasm32")]
mod wasm;

pub use humanoid::BONES;
pub use stl::readme_stl;
pub use vrma::vrma;

use glb::Glb;
use retarget::{Options, Rig};
use scene::Clip;
use serde_json::{json, Value};

/// What a file is: its bone map, its clips, and what can be done with it.
pub fn inspect(bytes: &[u8]) -> Result<Value, String> {
    let g = Glb::parse(bytes)?;
    let rig = Rig::read(&g)?;
    let clips = Clip::read_all(&g, rig.scene.len())?;
    let bones: serde_json::Map<String, Value> =
        rig.map.bones.iter().map(|(b, n)| (b.to_string(), json!(rig.scene.names[*n]))).collect();
    let required = ["hips", "spine", "head", "leftUpperArm", "leftLowerArm", "leftHand", "rightUpperArm", "rightLowerArm", "rightHand", "leftUpperLeg", "leftLowerLeg", "leftFoot", "rightUpperLeg", "rightLowerLeg", "rightFoot"];
    let missing: Vec<&str> = required.iter().copied().filter(|b| rig.map.get(b).is_none()).collect();
    let skinned = !g.list("skins").is_empty();
    let facing = rig.basis.rot(math::V3(0.0, 0.0, 1.0));
    Ok(json!({
        "bones": bones,
        "map": rig.map.how,
        "missing": missing,
        "humanoid": missing.is_empty(),
        "skinned": skinned,
        "meshes": g.list("meshes").len(),
        "nodes": rig.scene.len(),
        "hips_height": (rig.hips_height * 1000.0).round() / 1000.0,
        // Which way the body faces, so a viewer can stand in front of it.
        "facing": [facing.0, facing.1, facing.2],
        "clips": clips.iter().map(|c| json!({ "name": c.name, "seconds": (c.duration * 1000.0).round() / 1000.0 })).collect::<Vec<_>>(),
        "vrm": g.json.pointer("/extensions/VRMC_vrm").is_some() || g.json.pointer("/extensions/VRM").is_some(),
        "vrma": g.json.pointer("/extensions/VRMC_vrm_animation").is_some(),
    }))
}

/// Copy clips from `source` onto `target`. `clips` names them (empty: all of
/// them). Returns the target GLB with the new clips added after its own, and
/// a report of what went in.
pub fn retarget(target: &[u8], source: &[u8], clips: &[String], in_place: bool) -> Result<(Vec<u8>, Value), String> {
    let mut tg = Glb::parse(target)?;
    let sg = Glb::parse(source)?;
    let dst = Rig::read(&tg)?;
    let src = Rig::read(&sg)?;
    let all = Clip::read_all(&sg, src.scene.len())?;
    if all.is_empty() {
        return Err("the source file has no animation clips".into());
    }
    let chosen: Vec<&Clip> = if clips.is_empty() {
        all.iter().collect()
    } else {
        let mut v = Vec::new();
        for name in clips {
            v.push(all.iter().find(|c| &c.name == name).ok_or(format!("the source has no clip named {name:?}"))?);
        }
        v
    };
    let opt = Options { in_place };
    let mut report = Vec::new();
    for clip in chosen {
        let baked = retarget::bake(&src, clip, &dst, &opt)?;
        let at = retarget::write(&mut tg, &baked, &clip.name);
        let name = tg.json["animations"][at]["name"].clone();
        report.push(json!({ "name": name, "bones": baked.bones, "keys": baked.times.len() }));
    }
    let shared = src.shared(&dst);
    let out = tg.encode();
    let bytes = out.len();
    Ok((out, json!({ "clips": report, "shared": shared, "bytes": bytes })))
}

/// The clips of a file on its bare skeleton: no mesh, no skin, no textures.
/// `clips` names them (empty: all, and a file with none gives the skeleton
/// alone). What a clip library ships, and what the page retargets onto to
/// preview a clip without copying a model of many megabytes each time.
pub fn clips_only(bytes: &[u8], clips: &[String]) -> Result<Vec<u8>, String> {
    bare(bytes, Some(clips))
}

/// A file's bare skeleton and no clips: what a model's moves are baked onto
/// when they are kept beside it rather than in it (`elo stage`).
pub fn skeleton(bytes: &[u8]) -> Result<Vec<u8>, String> {
    bare(bytes, None)
}

fn bare(bytes: &[u8], keep: Option<&[String]>) -> Result<Vec<u8>, String> {
    let clips = keep.unwrap_or(&[]);
    let g = Glb::parse(bytes)?;
    let rig = Rig::read(&g)?;
    let s = &rig.scene;
    let all = Clip::read_all(&g, s.len())?;
    let mut nodes = Vec::with_capacity(s.len());
    for i in 0..s.len() {
        let r = &s.rest[i];
        let mut n = json!({
            "name": s.names[i],
            "translation": [r.t.0, r.t.1, r.t.2],
            "rotation": [r.r.0, r.r.1, r.r.2, r.r.3],
            "scale": [r.s.0, r.s.1, r.s.2],
        });
        if !s.children[i].is_empty() {
            n["children"] = json!(s.children[i]);
        }
        nodes.push(n);
    }
    let roots: Vec<usize> = (0..s.len()).filter(|i| s.parent[*i].is_none()).collect();
    let bones: serde_json::Map<String, Value> =
        rig.map.bones.iter().map(|(b, n)| (b.to_string(), json!({ "node": n }))).collect();
    let mut out = Glb {
        json: json!({
            "asset": { "version": "2.0", "generator": "Elo Pros rig desk" },
            "scene": 0,
            "scenes": [{ "nodes": roots }],
            "nodes": nodes,
            "extensionsUsed": ["VRMC_vrm_animation"],
            "extensions": { "VRMC_vrm_animation": { "specVersion": "1.0", "humanoid": { "humanBones": bones } } },
        }),
        bin: Vec::new(),
    };
    let mut anims = Vec::new();
    // Channels baked at one rate share their key times: write each list once.
    let mut inputs: std::collections::HashMap<Vec<u32>, usize> = std::collections::HashMap::new();
    for c in all.iter().filter(|c| keep.is_some() && (clips.is_empty() || clips.contains(&c.name))) {
        let (mut samplers, mut channels) = (Vec::new(), Vec::new());
        for ch in &c.channels {
            // A channel that never moves says one thing, or nothing if that
            // is where the node already rests.
            let first = &ch.values[..ch.values.len().min(1)];
            let still = ch.values.iter().all(|v| v.iter().zip(&ch.values[0]).all(|(a, b)| (a - b).abs() < 1e-6));
            let (times, values) = if still { (&ch.times[..ch.times.len().min(1)], first) } else { (&ch.times[..], &ch.values[..]) };
            if still {
                let r = &s.rest[ch.node];
                let rest: Vec<f64> = match ch.path {
                    scene::Path::T => vec![r.t.0, r.t.1, r.t.2],
                    scene::Path::R => vec![r.r.0, r.r.1, r.r.2, r.r.3],
                    scene::Path::S => vec![r.s.0, r.s.1, r.s.2],
                };
                let same = |v: &[f64]| v.iter().zip(&rest).all(|(a, b)| (a - b).abs() < 1e-5);
                let flipped: Vec<f64> = values.first().map(|v| v.iter().map(|x| -x).collect()).unwrap_or_default();
                if values.first().is_none_or(|v| same(v) || (ch.path == scene::Path::R && same(&flipped))) {
                    continue;
                }
            }
            let times: Vec<f32> = times.iter().map(|t| *t as f32).collect();
            let flat: Vec<f32> = values.iter().flatten().map(|v| *v as f32).collect();
            let (kind, path) = match ch.path {
                scene::Path::T => ("VEC3", "translation"),
                scene::Path::R => ("VEC4", "rotation"),
                scene::Path::S => ("VEC3", "scale"),
            };
            let key: Vec<u32> = times.iter().map(|t| t.to_bits()).collect();
            let input = match inputs.get(&key) {
                Some(i) => *i,
                None => {
                    let i = out.push_f32(&times, "SCALAR", true);
                    inputs.insert(key, i);
                    i
                }
            };
            let output = out.push_f32(&flat, kind, false);
            channels.push(json!({ "sampler": samplers.len(), "target": { "node": ch.node, "path": path } }));
            samplers.push(json!({ "input": input, "output": output, "interpolation": if ch.step { "STEP" } else { "LINEAR" } }));
        }
        anims.push(json!({ "name": c.name, "samplers": samplers, "channels": channels }));
    }
    if anims.is_empty() && !clips.is_empty() {
        return Err("none of those clips is in the file".into());
    }
    if !anims.is_empty() {
        out.json["animations"] = json!(anims);
    }
    Ok(out.encode())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_non_glb_is_refused_plainly() {
        assert!(super::inspect(b"hello").unwrap_err().contains("not a GLB"));
    }
}
