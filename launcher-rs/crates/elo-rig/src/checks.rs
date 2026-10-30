//! Measures a retarget instead of looking at it: after the clip is written and
//! read back, every mapped bone must point where the source bone points (in
//! the target's frame), frame by frame.

use crate::glb::Glb;
use crate::humanoid::BONES;
use crate::math::V3;
use crate::retarget::Rig;
use crate::scene::Clip;

/// The worst angle, in degrees, between a source bone's direction and the
/// target bone's, over the clip, with the bone that had it. The two files'
/// frames are stated by the caller (`yaw`, degrees about +Y from the source
/// to the target), never read from the rigs: a check that measured the frame
/// the way the retarget does would agree with any mistake in it.
pub fn worst_direction(source: &[u8], clip: &str, out: &[u8], out_clip: &str, yaw: f64) -> (f64, String) {
    let sg = Glb::parse(source).unwrap();
    let tg = Glb::parse(out).unwrap();
    let src = Rig::read(&sg).unwrap();
    let dst = Rig::read(&tg).unwrap();
    let sc = Clip::read_all(&sg, src.scene.len()).unwrap().into_iter().find(|c| c.name == clip).unwrap();
    let tc = Clip::read_all(&tg, dst.scene.len()).unwrap().into_iter().find(|c| c.name == out_clip).unwrap();
    let align = crate::math::Q::axis_angle(V3(0.0, 1.0, 0.0), yaw.to_radians());
    let mut worst = (0.0, String::new());
    for k in 0..=40 {
        let t = sc.duration * k as f64 / 40.0;
        let ws = src.scene.world(&sc.pose(&src.scene.rest, t));
        let wt = dst.scene.world(&tc.pose(&dst.scene.rest, t));
        for (b, toward) in BONES {
            let Some(c) = toward.iter().find(|c| src.map.get(c).is_some() && dst.map.get(c).is_some()) else { continue };
            let (Some(sb), Some(tb)) = (src.map.get(b), dst.map.get(b)) else { continue };
            let (sc_, tc_) = (src.map.get(c).unwrap(), dst.map.get(c).unwrap());
            let ds = align.rot(ws[sc_].origin().sub(ws[sb].origin()));
            let dt = wt[tc_].origin().sub(wt[tb].origin());
            let (Some(ds), Some(dt)) = (ds.norm(), dt.norm()) else { continue };
            let angle = ds.dot(dt).clamp(-1.0, 1.0).acos().to_degrees();
            if angle > worst.0 {
                worst = (angle, format!("{b} at {t:.2}s"));
            }
        }
    }
    worst
}

/// The worst difference, in degrees, between how far the source's head has
/// turned from its rest and how far the target's has, over the clip. A head
/// has no bone beyond it to point at, so its direction says nothing; its turn
/// is what a viewer sees.
pub fn worst_head_turn(source: &[u8], clip: &str, out: &[u8], out_clip: &str) -> f64 {
    let sg = Glb::parse(source).unwrap();
    let tg = Glb::parse(out).unwrap();
    let (src, dst) = (Rig::read(&sg).unwrap(), Rig::read(&tg).unwrap());
    let sc = Clip::read_all(&sg, src.scene.len()).unwrap().into_iter().find(|c| c.name == clip).unwrap();
    let tc = Clip::read_all(&tg, dst.scene.len()).unwrap().into_iter().find(|c| c.name == out_clip).unwrap();
    let (hs, ht) = (src.map.get("head").unwrap(), dst.map.get("head").unwrap());
    let mut worst: f64 = 0.0;
    for k in 0..=40 {
        let t = sc.duration * k as f64 / 40.0;
        let ws = src.scene.world_rot(&sc.pose(&src.scene.rest, t));
        let wt = dst.scene.world_rot(&tc.pose(&dst.scene.rest, t));
        let ds = ws[hs].mul(src.rest_q[hs].inv());
        let dt = wt[ht].mul(dst.rest_q[ht].inv());
        let off = ds.inv().mul(dt);
        worst = worst.max(2.0 * off.3.abs().min(1.0).acos().to_degrees());
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::worst_direction;
    use crate::glb::Glb;
    use crate::math::{Q, V3};
    use crate::scene::Clip;
    use serde_json::json;

    /// Two Mesh2Motion clips (CC0) on their bare skeleton, made by `clips_only`.
    const CLIPS: &[u8] = include_bytes!("../tests/fixtures/mesh2motion-walk-sit.glb");

    fn tarshish() -> Vec<u8> {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../agents/tarshish/stage-3d/tarshish-3d.glb");
        std::fs::read(p).expect("Tarshish's model is in the repo")
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_meshy_body_takes_mesh2motion_clips() {
        let (out, report) = crate::retarget(&tarshish(), CLIPS, &names(&["Walk", "Sitting_Idle"]), false).unwrap();
        assert_eq!(report["clips"][0]["bones"], 52, "{report}");
        for clip in ["Walk", "Sitting_Idle"] {
            let (deg, at) = worst_direction(CLIPS, clip, &out, clip, 0.0);
            assert!(deg < 0.5, "{clip}: {deg:.2}° at {at}");
            // Her head turns as the mannequin's does, never bowed by its neck.
            let head = super::worst_head_turn(CLIPS, clip, &out, clip);
            assert!(head < 0.5, "{clip}: head off by {head:.2}°");
        }
        // Her own five clips are still there, ahead of the new ones.
        let info = crate::inspect(&out).unwrap();
        assert_eq!(info["clips"].as_array().unwrap().len(), 7);
        assert_eq!(info["clips"][0]["name"], "Running");
    }

    #[test]
    fn a_rig_onto_itself_changes_nothing() {
        let (out, _) = crate::retarget(CLIPS, CLIPS, &names(&["Walk"]), false).unwrap();
        let (deg, at) = worst_direction(CLIPS, "Walk", &out, "Walk 2", 0.0);
        assert!(deg < 0.01, "{deg}° at {at}");
    }

    #[test]
    fn a_bare_clip_file_keeps_the_motion() {
        let bare = crate::clips_only(CLIPS, &names(&["Walk"])).unwrap();
        let (deg, at) = worst_direction(CLIPS, "Walk", &bare, "Walk", 0.0);
        assert!(deg < 0.01, "{deg}° at {at}");
        assert!(crate::clips_only(CLIPS, &names(&["Nope"])).is_err());
    }

    #[test]
    fn the_preview_skeleton_gets_the_same_clip_as_the_model() {
        // The page previews on the model's bare skeleton and downloads the
        // model itself: both must carry the same numbers.
        let model = tarshish();
        let skeleton = crate::clips_only(&model, &[]).unwrap();
        assert!(skeleton.len() < model.len() / 20);
        let read = |bytes: &[u8]| {
            let g = Glb::parse(bytes).unwrap();
            let n = g.list("nodes").len();
            Clip::read_all(&g, n).unwrap().into_iter().last().unwrap()
        };
        let a = read(&crate::retarget(&skeleton, CLIPS, &names(&["Walk"]), false).unwrap().0);
        let b = read(&crate::retarget(&model, CLIPS, &names(&["Walk"]), false).unwrap().0);
        assert_eq!(a.channels.len(), b.channels.len());
        for (x, y) in a.channels.iter().zip(&b.channels) {
            assert_eq!((x.node, x.path), (y.node, y.path));
            assert_eq!(x.values, y.values);
        }
        let bare = Glb::parse(&crate::skeleton(&model).unwrap()).unwrap();
        assert!(bare.list("animations").is_empty() && bare.list("nodes").len() == Glb::parse(&model).unwrap().list("nodes").len());
        // A file with no clips still gives its skeleton.
        let bare = crate::clips_only(&crate::vrma(CLIPS, "Walk", false).unwrap(), &[]).unwrap();
        let g = Glb::parse(&crate::clips_only(&bare, &[]).unwrap()).unwrap();
        assert!(!g.list("nodes").is_empty());
    }

    /// A VRM 0 faces -Z and states its own bone map with 0.x thumb names.
    fn as_vrm0(bytes: &[u8]) -> Vec<u8> {
        let mut g = Glb::parse(bytes).unwrap();
        let info = crate::inspect(bytes).unwrap();
        let nodes = g.json["nodes"].as_array().unwrap().clone();
        let index = |name: &str| nodes.iter().position(|n| n["name"] == name).unwrap();
        let bones: Vec<_> = info["bones"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(b, n)| {
                let b = b.replace("ThumbProximal", "ThumbIntermediate").replace("ThumbMetacarpal", "ThumbProximal");
                json!({ "bone": b, "node": index(n.as_str().unwrap()) })
            })
            .collect();
        let roots = g.json["scenes"][0]["nodes"].clone();
        let turn = Q::axis_angle(V3(0.0, 1.0, 0.0), std::f64::consts::PI);
        let n = nodes.len();
        g.json["nodes"].as_array_mut().unwrap().push(json!({ "name": "turned", "rotation": [turn.0, turn.1, turn.2, turn.3], "children": roots }));
        g.json["scenes"][0]["nodes"] = json!([n]);
        g.json["animations"] = json!([]);
        g.json["extensions"] = json!({ "VRM": { "humanoid": { "humanBones": bones } } });
        g.encode()
    }

    #[test]
    fn a_vrm0_facing_back_gets_the_clip_the_right_way_round() {
        let vrm = as_vrm0(CLIPS);
        assert_eq!(crate::inspect(&vrm).unwrap()["map"], "vrm0");
        let (out, _) = crate::retarget(&vrm, CLIPS, &names(&["Walk"]), false).unwrap();
        let (deg, at) = worst_direction(CLIPS, "Walk", &out, "Walk", 180.0);
        assert!(deg < 0.5, "{deg}° at {at}");
    }

    #[test]
    fn a_vrma_rests_in_a_t_pose_and_carries_the_clip() {
        let vrma = crate::vrma(CLIPS, "Walk", false).unwrap();
        let info = crate::inspect(&vrma).unwrap();
        assert_eq!(info["map"], "vrma");
        assert_eq!(info["vrma"], true);
        assert_eq!(info["missing"], json!([]));
        let g = Glb::parse(&vrma).unwrap();
        assert_eq!(g.json["extensionsUsed"], json!(["VRMC_vrm_animation"]));
        for n in g.json["nodes"].as_array().unwrap() {
            assert!(n.get("rotation").is_none(), "{n}");
        }
        let (deg, at) = worst_direction(CLIPS, "Walk", &vrma, "Walk", 0.0);
        assert!(deg < 0.5, "{deg}° at {at}");
        // And a VRMA is a source like any other.
        let (out, _) = crate::retarget(&tarshish(), &vrma, &[], false).unwrap();
        let (deg, at) = worst_direction(CLIPS, "Walk", &out, "Walk", 0.0);
        assert!(deg < 0.5, "{deg}° at {at}");
    }

    #[test]
    fn in_place_keeps_the_hips_over_one_spot() {
        let (out, _) = crate::retarget(&tarshish(), CLIPS, &names(&["Walk"]), true).unwrap();
        let g = Glb::parse(&out).unwrap();
        let s = crate::scene::Scene::read(&g).unwrap();
        let clip = Clip::read_all(&g, s.len()).unwrap().into_iter().find(|c| c.name == "Walk").unwrap();
        let hips = s.names.iter().position(|n| n == "spine").unwrap();
        let rest = s.world(&s.rest)[hips].origin();
        let mut lift: f64 = 0.0;
        for k in 0..20 {
            let at = s.world(&clip.pose(&s.rest, clip.duration * k as f64 / 20.0))[hips].origin();
            assert!((at.0 - rest.0).abs() < 1e-4 && (at.2 - rest.2).abs() < 1e-4, "{at:?} vs {rest:?}");
            lift = lift.max((at.1 - rest.1).abs());
        }
        assert!(lift > 0.001, "a walk bobs");
    }

    #[test]
    fn a_readme_block_fits_github() {
        let md = crate::readme_stl(&tarshish(), 1500, "tarshish").unwrap();
        assert!(md.starts_with("```stl\nsolid tarshish\n") && md.ends_with("endsolid tarshish\n```\n"));
        let facets = md.matches("facet normal").count();
        assert!((700..=1500).contains(&facets), "{facets}");
        assert!(md.len() < 512 * 1024, "{}", md.len());
        // Z-up, on the floor, 100 tall.
        let zs: Vec<f64> = md.lines().filter_map(|l| l.trim().strip_prefix("vertex ")).map(|v| v.split(' ').nth(2).unwrap().parse().unwrap()).collect();
        let (lo, hi) = zs.iter().fold((f64::MAX, f64::MIN), |(a, b), z| (a.min(*z), b.max(*z)));
        assert!(lo.abs() < 0.5 && (hi - 100.0).abs() < 2.0, "{lo} {hi}");
    }
}
