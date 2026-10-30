//! A clip as a VRM Animation (`VRMC_vrm_animation` 1.0): a bare skeleton of
//! humanoid bones, a clip on it, and the map from bone names to nodes.
//!
//! Players apply a VRMA by each bone's turn from the file's rest, onto a VRM's
//! normalised T-pose, so the skeleton written here rests in a T-pose with no
//! rotation on any joint: spine up, arms out to the sides, legs down, facing
//! +Z with its left on +X. Its lengths are the source rig's, and its hips
//! stand at the source's hip height, which players use to scale the travel.

use crate::glb::Glb;
use crate::humanoid::{canonical_parent, BONES};
use crate::math::{Q, V3};
use crate::retarget::{self, Options, Rig};
use crate::scene::Clip;
use serde_json::{json, Map, Value};

pub fn vrma(source: &[u8], clip: &str, in_place: bool) -> Result<Vec<u8>, String> {
    let sg = Glb::parse(source)?;
    let src = Rig::read(&sg)?;
    let clips = Clip::read_all(&sg, src.scene.len())?;
    let chosen = clips.iter().find(|c| c.name == clip).ok_or(format!("the source has no clip named {clip:?}"))?;
    let mut out = skeleton(&src)?;
    let dst = Rig::read(&out)?;
    let baked = retarget::bake(&src, chosen, &dst, &Options { in_place })?;
    retarget::write(&mut out, &baked, &chosen.name);
    Ok(out.encode())
}

/// The nearest bone above `b` that the source has.
fn present_parent(src: &Rig, b: &str) -> Option<&'static str> {
    let mut p = canonical_parent(b);
    while let Some(pb) = p {
        if src.map.get(pb).is_some() {
            return Some(pb);
        }
        p = canonical_parent(pb);
    }
    None
}

fn skeleton(src: &Rig) -> Result<Glb, String> {
    if src.map.get("hips").is_none() {
        return Err("the source has no hips to build a VRM Animation from".into());
    }
    let to_frame = src.basis.inv();
    let mut nodes: Vec<Value> = Vec::new();
    let mut index: Vec<(&str, usize)> = Vec::new();
    for (b, _) in BONES {
        if src.map.get(b).is_none() {
            continue;
        }
        let offset = match present_parent(src, b) {
            None => V3(0.0, src.hips_height, 0.0),
            Some(p) => {
                let actual = to_frame.rot(src.pos(b).unwrap().sub(src.pos(p).unwrap()));
                let len = actual.len();
                let x = if b.starts_with("left") { 1.0 } else { -1.0 };
                let part = b.trim_start_matches("left").trim_start_matches("right");
                let dir = match part {
                    "spine" | "chest" | "upperChest" | "neck" | "head" => V3(0.0, 1.0, 0.0),
                    "UpperArm" | "LowerArm" | "Hand" => V3(x, 0.0, 0.0),
                    "LowerLeg" | "Foot" => V3(0.0, -1.0, 0.0),
                    p if p.starts_with("Thumb") && p != "ThumbMetacarpal" => V3(x * std::f64::consts::FRAC_1_SQRT_2, 0.0, std::f64::consts::FRAC_1_SQRT_2),
                    p if p.ends_with("Intermediate") || p.ends_with("Distal") => V3(x, 0.0, 0.0),
                    // Knuckles keep their spread, turned with the forearm
                    // from wherever it rests onto the straight T-pose arm.
                    p if p.ends_with("Proximal") || p == "ThumbMetacarpal" => {
                        let side = if x > 0.0 { "left" } else { "right" };
                        let arm = src
                            .pos(&format!("{side}Hand"))
                            .zip(src.pos(&format!("{side}LowerArm")))
                            .map(|(h, l)| to_frame.rot(h.sub(l)))
                            .unwrap_or(V3(x, 0.0, 0.0));
                        let k = Q::arc(arm, V3(x, 0.0, 0.0)).rot(actual);
                        index.push((b, nodes.len()));
                        nodes.push(json!({ "name": b, "translation": [k.0, k.1, k.2] }));
                        continue;
                    }
                    // Shoulders, hips to legs, the knuckles and the toes keep
                    // the source's own offsets: they are where a T-pose puts them.
                    _ => {
                        index.push((b, nodes.len()));
                        nodes.push(json!({ "name": b, "translation": [actual.0, actual.1, actual.2] }));
                        continue;
                    }
                };
                dir.scale(len)
            }
        };
        index.push((b, nodes.len()));
        nodes.push(json!({ "name": b, "translation": [offset.0, offset.1, offset.2] }));
    }
    for (b, i) in &index {
        if let Some(p) = present_parent(src, b) {
            let pi = index.iter().find(|(x, _)| *x == p).map(|(_, n)| *n).unwrap();
            let kids = nodes[pi].as_object_mut().unwrap().entry("children").or_insert_with(|| json!([]));
            kids.as_array_mut().unwrap().push(json!(i));
        }
    }
    let hips = index.iter().find(|(b, _)| *b == "hips").map(|(_, n)| *n).unwrap();
    let mut bones = Map::new();
    for (b, i) in &index {
        bones.insert(b.to_string(), json!({ "node": i }));
    }
    Ok(Glb {
        json: json!({
            "asset": { "version": "2.0", "generator": "Elo Pros rig desk" },
            "scene": 0,
            "scenes": [{ "nodes": [hips] }],
            "nodes": nodes,
            "extensionsUsed": ["VRMC_vrm_animation"],
            "extensions": { "VRMC_vrm_animation": { "specVersion": "1.0", "humanoid": { "humanBones": bones } } },
        }),
        bin: Vec::new(),
    })
}
