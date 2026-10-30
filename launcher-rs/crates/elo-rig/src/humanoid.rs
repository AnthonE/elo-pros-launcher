//! Which node is which bone. The bone names are VRM 1.0's humanoid names
//! (`hips`, `leftUpperArm`, …), the open standard a VTuber model already
//! carries. A VRM or VRMA file states its map; anything else is read from its
//! node names and its shape.
//!
//! Names alone are not enough: Rigify and Meshy call the hips `spine` and the
//! head `spine.005`, Daz and Character Creator have two pelvis bones, and a
//! spine can be two bones or five. So only the limbs are found by name; the
//! hips are where the legs meet the path to the head, and the spine, chest and
//! neck are cut from that path at the bone the shoulders hang from.

use crate::glb::Glb;
use crate::scene::Scene;
use serde_json::Value;
use std::collections::BTreeMap;

/// Every bone this crate maps, parents before children, with the bone whose
/// position gives its direction (the first one present).
pub const BONES: &[(&str, &[&str])] = &[
    ("hips", &[]),
    ("spine", &["chest", "upperChest", "neck", "head"]),
    ("chest", &["upperChest", "neck", "head"]),
    ("upperChest", &["neck", "head"]),
    ("neck", &["head"]),
    ("head", &[]),
    ("leftShoulder", &["leftUpperArm"]),
    ("leftUpperArm", &["leftLowerArm"]),
    ("leftLowerArm", &["leftHand"]),
    ("leftHand", &["leftMiddleProximal", "leftIndexProximal", "leftRingProximal"]),
    ("rightShoulder", &["rightUpperArm"]),
    ("rightUpperArm", &["rightLowerArm"]),
    ("rightLowerArm", &["rightHand"]),
    ("rightHand", &["rightMiddleProximal", "rightIndexProximal", "rightRingProximal"]),
    ("leftUpperLeg", &["leftLowerLeg"]),
    ("leftLowerLeg", &["leftFoot"]),
    ("leftFoot", &["leftToes"]),
    ("leftToes", &[]),
    ("rightUpperLeg", &["rightLowerLeg"]),
    ("rightLowerLeg", &["rightFoot"]),
    ("rightFoot", &["rightToes"]),
    ("rightToes", &[]),
    ("leftThumbMetacarpal", &["leftThumbProximal"]),
    ("leftThumbProximal", &["leftThumbDistal"]),
    ("leftThumbDistal", &[]),
    ("leftIndexProximal", &["leftIndexIntermediate"]),
    ("leftIndexIntermediate", &["leftIndexDistal"]),
    ("leftIndexDistal", &[]),
    ("leftMiddleProximal", &["leftMiddleIntermediate"]),
    ("leftMiddleIntermediate", &["leftMiddleDistal"]),
    ("leftMiddleDistal", &[]),
    ("leftRingProximal", &["leftRingIntermediate"]),
    ("leftRingIntermediate", &["leftRingDistal"]),
    ("leftRingDistal", &[]),
    ("leftLittleProximal", &["leftLittleIntermediate"]),
    ("leftLittleIntermediate", &["leftLittleDistal"]),
    ("leftLittleDistal", &[]),
    ("rightThumbMetacarpal", &["rightThumbProximal"]),
    ("rightThumbProximal", &["rightThumbDistal"]),
    ("rightThumbDistal", &[]),
    ("rightIndexProximal", &["rightIndexIntermediate"]),
    ("rightIndexIntermediate", &["rightIndexDistal"]),
    ("rightIndexDistal", &[]),
    ("rightMiddleProximal", &["rightMiddleIntermediate"]),
    ("rightMiddleIntermediate", &["rightMiddleDistal"]),
    ("rightMiddleDistal", &[]),
    ("rightRingProximal", &["rightRingIntermediate"]),
    ("rightRingIntermediate", &["rightRingDistal"]),
    ("rightRingDistal", &[]),
    ("rightLittleProximal", &["rightLittleIntermediate"]),
    ("rightLittleIntermediate", &["rightLittleDistal"]),
    ("rightLittleDistal", &[]),
];

/// The canonical parent of a bone: the nearest earlier bone it hangs from.
pub fn canonical_parent(bone: &str) -> Option<&'static str> {
    let side = if bone.starts_with("left") { "left" } else if bone.starts_with("right") { "right" } else { "" };
    let part = &bone[side.len()..];
    let s = |p: &str| -> &'static str {
        BONES.iter().map(|(b, _)| *b).find(|b| b.len() == side.len() + p.len() && b.starts_with(side) && b.ends_with(p)).unwrap_or("hips")
    };
    Some(match part {
        "hips" => return None,
        "spine" => "hips",
        "chest" => "spine",
        "upperChest" => "chest",
        "neck" => "upperChest",
        "head" => "neck",
        "Shoulder" => "upperChest",
        "UpperArm" => s("Shoulder"),
        "LowerArm" => s("UpperArm"),
        "Hand" => s("LowerArm"),
        "UpperLeg" => "hips",
        "LowerLeg" => s("UpperLeg"),
        "Foot" => s("LowerLeg"),
        "Toes" => s("Foot"),
        _ => {
            // A finger joint hangs from the joint before it, the first from the hand.
            const JOINTS: [&str; 4] = ["Metacarpal", "Proximal", "Intermediate", "Distal"];
            let k = JOINTS.iter().position(|j| part.ends_with(j))?;
            let finger = &part[..part.len() - JOINTS[k].len()];
            return JOINTS[..k]
                .iter()
                .rev()
                .find_map(|j| canonical(&format!("{side}{finger}{j}")))
                .or(Some(s("Hand")));
        }
    })
}

#[derive(Clone, Debug, Default)]
pub struct HumanMap {
    pub bones: BTreeMap<&'static str, usize>,
    /// Where the map came from: `vrm1`, `vrm0`, `vrma` or `names`.
    pub how: &'static str,
}

impl HumanMap {
    pub fn get(&self, bone: &str) -> Option<usize> {
        self.bones.get(bone).copied()
    }

    pub fn read(g: &Glb, s: &Scene) -> HumanMap {
        if let Some(m) = from_extension(g, s) {
            return m;
        }
        from_names(s)
    }
}

fn canonical(name: &str) -> Option<&'static str> {
    BONES.iter().map(|(b, _)| *b).find(|b| *b == name)
}

fn from_extension(g: &Glb, s: &Scene) -> Option<HumanMap> {
    let ext = g.json.get("extensions")?;
    let mut bones = BTreeMap::new();
    let mut put = |name: &str, node: Option<u64>| {
        if let (Some(b), Some(n)) = (canonical(name), node) {
            if (n as usize) < s.len() {
                bones.insert(b, n as usize);
            }
        }
    };
    let how;
    if let Some(h) = ext.pointer("/VRMC_vrm/humanoid/humanBones").and_then(Value::as_object) {
        how = "vrm1";
        for (k, v) in h {
            put(k, v.get("node").and_then(Value::as_u64));
        }
    } else if let Some(h) = ext.pointer("/VRMC_vrm_animation/humanoid/humanBones").and_then(Value::as_object) {
        how = "vrma";
        for (k, v) in h {
            put(k, v.get("node").and_then(Value::as_u64));
        }
    } else {
        let h = ext.pointer("/VRM/humanoid/humanBones").and_then(Value::as_array)?;
        how = "vrm0";
        for v in h {
            let Some(bone) = v.get("bone").and_then(Value::as_str) else { continue };
            // VRM 0.x counts the thumb from one joint further in.
            let bone = bone
                .replace("ThumbProximal", "ThumbMetacarpal")
                .replace("ThumbIntermediate", "ThumbProximal");
            put(&bone, v.get("node").and_then(Value::as_u64));
        }
    }
    bones.contains_key("hips").then_some(HumanMap { bones, how })
}

// ── names ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    L,
    R,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Role {
    Head,
    Neck,
    Spine,
    Hips,
    Shoulder,
    UpperArm,
    LowerArm,
    Hand,
    UpperLeg,
    LowerLeg,
    Foot,
    Toes,
    /// Finger 0..5 (thumb, index, middle, ring, little), joint 0..3 from the hand.
    Finger(usize, usize),
}

/// Lowercase tokens: split at separators, at lower→upper and letter↔digit.
fn tokens(name: &str) -> Vec<String> {
    let name = name.rsplit(['|', ':']).next().unwrap_or(name);
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        let split = match prev {
            _ if !c.is_alphanumeric() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                prev = None;
                continue;
            }
            Some(p) => (p.is_lowercase() && c.is_uppercase()) || (p.is_ascii_digit() != c.is_ascii_digit()),
            None => false,
        };
        if split && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(c);
        prev = Some(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|t| t.to_lowercase()).collect()
}

/// A node with any of these is not a body bone: ends, twists, controls,
/// secondary motion, the face.
const SKIP: &[&str] = &[
    "end", "leaf", "nub", "tip", "top", "twist", "roll", "ik", "fk", "pole", "target", "mch", "sec", "helper", "ctrl",
    "control", "jiggle", "share", "tweak", "socket", "weapon", "prop", "attach", "breast", "bust", "hair", "skirt",
    "sleeve", "cloth", "eye", "jaw", "tongue", "teeth", "brow", "lid", "lip", "cheek", "nose", "ear", "palm", "carpal",
    "metacarpal", "bend2", "front", "back", "null", "dummy", "adj", "correct", "corrective",
];
/// Tokens that say nothing: rig prefixes and the VRoid centre mark.
const NOISE: &[&str] = &["mixamorig", "def", "org", "cc", "base", "j", "bip", "bip01", "bip001", "c", "f", "armature", "rig", "bone", "b"];

fn classify(name: &str) -> Option<(Option<Side>, Role, bool)> {
    let toks = tokens(name);
    let def = toks.first().is_some_and(|t| t == "def");
    let mut side = None;
    let mut words = Vec::new();
    let mut number = None;
    for t in &toks {
        match t.as_str() {
            "l" | "left" | "lf" | "lft" => side = Some(Side::L),
            "r" | "right" | "rt" | "rgt" => side = Some(Side::R),
            t if t.chars().all(|c| c.is_ascii_digit()) => number = t.parse::<usize>().ok(),
            t if NOISE.contains(&t) => {}
            t => words.push(t.to_string()),
        }
    }
    // Mixamo's "mixamorigLeftArm" survives as one token when the colon was dropped.
    if let Some(w) = words.first_mut() {
        if let Some(rest) = w.strip_prefix("mixamorig") {
            *w = rest.to_string();
        }
    }
    if words.iter().any(|w| SKIP.contains(&w.as_str())) {
        return None;
    }
    let finger = |w: &str| match w {
        "thumb" => Some(0),
        "index" | "pointer" => Some(1),
        "middle" | "mid" => Some(2),
        "ring" => Some(3),
        "pinky" | "little" | "small" => Some(4),
        _ => None,
    };
    if let Some(f) = words.iter().find_map(|w| finger(w)) {
        let seg = if words.iter().any(|w| w == "metacarpal") {
            Some(0)
        } else if words.iter().any(|w| w == "proximal") {
            Some(if f == 0 { 1 } else { 0 })
        } else if words.iter().any(|w| w == "intermediate") {
            Some(1)
        } else if words.iter().any(|w| w == "distal") {
            Some(2)
        } else {
            number.filter(|n| (1..=3).contains(n)).map(|n| n - 1)
        }?;
        return Some((side, Role::Finger(f, seg), def));
    }
    let word: String = words.concat();
    let role = match word.as_str() {
        "head" => Role::Head,
        w if w.starts_with("neck") => Role::Neck,
        w if w.starts_with("spine") || w.starts_with("chest") || w.starts_with("abdomen") || w == "waist" || w == "torso" => Role::Spine,
        "hips" | "hip" | "pelvis" => Role::Hips,
        "shoulder" | "clavicle" | "collar" | "collarbone" => Role::Shoulder,
        "upperarm" | "arm" | "uparm" | "shldrbend" | "shldr" => Role::UpperArm,
        "forearm" | "lowerarm" | "loarm" | "elbow" | "forearmbend" => Role::LowerArm,
        "hand" | "wrist" => Role::Hand,
        "upleg" | "upperleg" | "thigh" | "thighbend" => Role::UpperLeg,
        "leg" | "lowerleg" | "loleg" | "calf" | "shin" | "knee" => Role::LowerLeg,
        "foot" | "ankle" => Role::Foot,
        "toe" | "toes" | "toebase" | "ball" => Role::Toes,
        _ => return None,
    };
    Some((side, role, def))
}

fn side_name(s: Side) -> &'static str {
    if s == Side::L { "left" } else { "right" }
}

fn bone_name(side: Side, part: &str) -> &'static str {
    let full = format!("{}{}", side_name(side), part);
    canonical(&full).expect("a known bone")
}

fn lca(s: &Scene, nodes: &[usize]) -> Option<usize> {
    let mut it = nodes.iter();
    let first = *it.next()?;
    let mut chain: Vec<usize> = std::iter::successors(Some(first), |n| s.parent[*n]).collect();
    for &n in it {
        let up: Vec<usize> = std::iter::successors(Some(n), |x| s.parent[*x]).collect();
        chain.retain(|a| up.contains(a));
    }
    chain.first().copied()
}

fn depth(s: &Scene, mut n: usize) -> usize {
    let mut d = 0;
    while let Some(p) = s.parent[n] {
        d += 1;
        n = p;
    }
    d
}

fn from_names(s: &Scene) -> HumanMap {
    let any_joint = s.joint.iter().any(|j| *j);
    let mut found: Vec<(usize, Option<Side>, Role, bool)> = Vec::new();
    for i in 0..s.len() {
        if any_joint && !s.joint[i] {
            continue;
        }
        if let Some((side, role, def)) = classify(&s.names[i]) {
            found.push((i, side, role, def));
        }
    }
    // The best node for a role: a DEF- bone, then the shallowest.
    let best = |side: Option<Side>, role: Role, under: Option<usize>| -> Option<usize> {
        found
            .iter()
            .filter(|(i, sd, r, _)| *r == role && *sd == side && under.is_none_or(|u| s.is_ancestor(u, *i)))
            .min_by_key(|(i, _, _, def)| (!*def, depth(s, *i)))
            .map(|(i, ..)| *i)
    };
    let mut bones: BTreeMap<&'static str, usize> = BTreeMap::new();

    let legs = (best(Some(Side::L), Role::UpperLeg, None), best(Some(Side::R), Role::UpperLeg, None));
    let mut head = best(None, Role::Head, None);
    if head.is_none() {
        // Rigify and Meshy: the head is the last of the spine.NNN chain.
        let start = match legs {
            (Some(l), Some(r)) => lca(s, &[l, r]),
            _ => best(None, Role::Hips, None),
        };
        let mut at = start;
        while let Some(n) = at {
            let next = s.children[n].iter().copied().find(|c| {
                found.iter().any(|(i, side, r, _)| i == c && side.is_none() && matches!(r, Role::Spine | Role::Neck))
            });
            if next.is_none() {
                break;
            }
            at = next;
        }
        head = at.filter(|h| Some(*h) != start);
    }
    let hips = match (legs, head) {
        ((Some(l), Some(r)), Some(h)) => lca(s, &[l, r, h]),
        ((Some(l), Some(r)), None) => lca(s, &[l, r]),
        _ => best(None, Role::Hips, None),
    };
    let Some(hips) = hips else { return HumanMap { bones, how: "names" } };
    bones.insert("hips", hips);

    for side in [Side::L, Side::R] {
        let shoulder = best(Some(side), Role::Shoulder, Some(hips));
        let upper = best(Some(side), Role::UpperArm, Some(shoulder.unwrap_or(hips)));
        let lower = upper.and_then(|u| best(Some(side), Role::LowerArm, Some(u)));
        let hand = lower.and_then(|l| best(Some(side), Role::Hand, Some(l)));
        for (part, n) in [("Shoulder", shoulder), ("UpperArm", upper), ("LowerArm", lower), ("Hand", hand)] {
            if let Some(n) = n {
                bones.insert(bone_name(side, part), n);
            }
        }
        let thigh = if side == Side::L { legs.0 } else { legs.1 }.filter(|t| s.is_ancestor(hips, *t));
        let shin = thigh.and_then(|t| best(Some(side), Role::LowerLeg, Some(t)));
        let foot = shin.and_then(|l| best(Some(side), Role::Foot, Some(l)));
        let toes = foot.and_then(|f| best(Some(side), Role::Toes, Some(f)));
        for (part, n) in [("UpperLeg", thigh), ("LowerLeg", shin), ("Foot", foot), ("Toes", toes)] {
            if let Some(n) = n {
                bones.insert(bone_name(side, part), n);
            }
        }
        if let Some(hand) = hand {
            const FINGERS: [&str; 5] = ["Thumb", "Index", "Middle", "Ring", "Little"];
            for (f, fname) in FINGERS.iter().enumerate() {
                let joints: &[&str] = if f == 0 { &["Metacarpal", "Proximal", "Distal"] } else { &["Proximal", "Intermediate", "Distal"] };
                let mut under = hand;
                for (seg, j) in joints.iter().enumerate() {
                    let Some(n) = best(Some(side), Role::Finger(f, seg), Some(under)) else { break };
                    bones.insert(bone_name(side, &format!("{fname}{j}")), n);
                    under = n;
                }
            }
        }
    }

    if let Some(head) = head.filter(|h| s.is_ancestor(hips, *h)) {
        bones.insert("head", head);
        // The path from the hips to the head, both left out.
        let mut path: Vec<usize> = std::iter::successors(s.parent[head], |n| s.parent[*n]).take_while(|n| *n != hips).collect();
        path.reverse();
        let arms: Vec<usize> = ["leftShoulder", "rightShoulder", "leftUpperArm", "rightUpperArm"]
            .iter()
            .filter_map(|b| bones.get(b).copied())
            .collect();
        let anchor = if arms.len() >= 2 {
            let l = bones.get("leftShoulder").or(bones.get("leftUpperArm")).copied();
            let r = bones.get("rightShoulder").or(bones.get("rightUpperArm")).copied();
            l.zip(r).and_then(|(l, r)| lca(s, &[l, r]))
        } else {
            None
        };
        let cut = match anchor.and_then(|a| path.iter().position(|n| *n == a)) {
            Some(k) => k + 1,
            None if path.len() >= 2 => path.len() - 1,
            None => path.len(),
        };
        let (spine, neck) = path.split_at(cut);
        if let Some(n) = neck.first() {
            bones.insert("neck", *n);
        }
        match spine.len() {
            0 => {}
            1 => {
                bones.insert("spine", spine[0]);
            }
            2 => {
                bones.insert("spine", spine[0]);
                bones.insert("chest", spine[1]);
            }
            k => {
                bones.insert("spine", spine[0]);
                bones.insert("chest", spine[k - 2]);
                bones.insert("upperChest", spine[k - 1]);
            }
        }
    }
    HumanMap { bones, how: "names" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_split_the_usual_ways() {
        assert_eq!(tokens("mixamorig:LeftHandIndex1"), ["left", "hand", "index", "1"]);
        assert_eq!(tokens("upper_arm.L"), ["upper", "arm", "l"]);
        assert_eq!(tokens("J_Bip_L_UpperArm"), ["j", "bip", "l", "upper", "arm"]);
        assert_eq!(tokens("lShldrBend"), ["l", "shldr", "bend"]);
        assert_eq!(tokens("spine.005"), ["spine", "005"]);
    }

    #[test]
    fn limbs_by_name() {
        let c = |n| classify(n).map(|(s, r, _)| (s, r));
        assert_eq!(c("mixamorig:LeftArm"), Some((Some(Side::L), Role::UpperArm)));
        assert_eq!(c("mixamorig:LeftForeArm"), Some((Some(Side::L), Role::LowerArm)));
        assert_eq!(c("mixamorig:RightUpLeg"), Some((Some(Side::R), Role::UpperLeg)));
        assert_eq!(c("mixamorig:RightLeg"), Some((Some(Side::R), Role::LowerLeg)));
        assert_eq!(c("upperarm_l"), Some((Some(Side::L), Role::UpperArm)));
        assert_eq!(c("calf_r"), Some((Some(Side::R), Role::LowerLeg)));
        assert_eq!(c("ball_l"), Some((Some(Side::L), Role::Toes)));
        assert_eq!(c("shin.L"), Some((Some(Side::L), Role::LowerLeg)));
        assert_eq!(c("J_Bip_R_LowerArm"), Some((Some(Side::R), Role::LowerArm)));
        assert_eq!(c("CC_Base_L_Upperarm"), Some((Some(Side::L), Role::UpperArm)));
        assert_eq!(c("lShldrBend"), Some((Some(Side::L), Role::UpperArm)));
        assert_eq!(c("lThighBend"), Some((Some(Side::L), Role::UpperLeg)));
        assert_eq!(c("Bip01 L Thigh"), Some((Some(Side::L), Role::UpperLeg)));
        assert_eq!(c("thumb.01.L"), Some((Some(Side::L), Role::Finger(0, 0))));
        assert_eq!(c("f_index.02.R"), Some((Some(Side::R), Role::Finger(1, 1))));
        assert_eq!(c("mixamorig:LeftHandPinky3"), Some((Some(Side::L), Role::Finger(4, 2))));
        assert_eq!(c("index_01_l"), Some((Some(Side::L), Role::Finger(1, 0))));
        assert_eq!(c("LeftThumbProximal"), Some((Some(Side::L), Role::Finger(0, 1))));
        for skip in ["mixamorig:HeadTop_End", "upperarm_twist_01_l", "index_04_leaf_l", "lShldrTwist", "headfront", "J_Sec_Hair1_01", "Bone_006", "index_metacarpal_l"] {
            assert_eq!(c(skip), None, "{skip}");
        }
    }

    #[test]
    fn every_bone_but_the_hips_has_a_parent() {
        for (b, _) in BONES {
            let p = canonical_parent(b);
            assert_eq!(p.is_none(), *b == "hips", "{b}");
            if let Some(p) = p {
                let (bi, pi) = (BONES.iter().position(|x| x.0 == *b), BONES.iter().position(|x| x.0 == p));
                assert!(pi < bi, "{p} must come before {b}");
            }
        }
        assert_eq!(canonical_parent("leftIndexDistal"), Some("leftIndexIntermediate"));
        assert_eq!(canonical_parent("leftThumbDistal"), Some("leftThumbProximal"));
        assert_eq!(canonical_parent("rightUpperArm"), Some("rightShoulder"));
    }
}
