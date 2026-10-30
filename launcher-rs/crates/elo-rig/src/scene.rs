//! The node tree of a glTF at rest, and its animations as samplers.

use crate::glb::Glb;
use crate::math::{M4, Q, V3};
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
pub struct Trs {
    pub t: V3,
    pub r: Q,
    pub s: V3,
}

impl Trs {
    pub fn m4(&self) -> M4 {
        M4::trs(self.t, self.r, self.s)
    }
}

pub struct Scene {
    pub names: Vec<String>,
    pub parent: Vec<Option<usize>>,
    pub children: Vec<Vec<usize>>,
    pub rest: Vec<Trs>,
    /// Parents before children.
    pub order: Vec<usize>,
    /// Nodes any skin uses as a joint.
    pub joint: Vec<bool>,
}

impl Scene {
    pub fn read(g: &Glb) -> Result<Scene, String> {
        let nodes = g.list("nodes");
        let n = nodes.len();
        if n == 0 {
            return Err("the file has no nodes".into());
        }
        let mut s = Scene {
            names: Vec::with_capacity(n),
            parent: vec![None; n],
            children: vec![vec![]; n],
            rest: Vec::with_capacity(n),
            order: Vec::with_capacity(n),
            joint: vec![false; n],
        };
        for (i, nd) in nodes.iter().enumerate() {
            s.names.push(nd.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("node{i}")));
            s.rest.push(node_trs(nd));
            for c in nd.get("children").and_then(Value::as_array).into_iter().flatten() {
                let c = c.as_u64().unwrap_or(u64::MAX) as usize;
                if c >= n || s.parent[c].is_some() || c == i {
                    return Err(format!("node {i} lists child {c}, which is not a tree"));
                }
                s.parent[c] = Some(i);
                s.children[i].push(c);
            }
        }
        // Roots first, then breadth by breadth.
        let mut queue: Vec<usize> = (0..n).filter(|i| s.parent[*i].is_none()).collect();
        let mut seen = vec![false; n];
        while let Some(i) = queue.first().copied() {
            queue.remove(0);
            if seen[i] {
                return Err("the node tree has a cycle".into());
            }
            seen[i] = true;
            s.order.push(i);
            queue.extend(s.children[i].iter().copied());
        }
        if s.order.len() != n {
            return Err("the node tree has a cycle".into());
        }
        for skin in g.list("skins") {
            for j in skin.get("joints").and_then(Value::as_array).into_iter().flatten() {
                if let Some(j) = j.as_u64().filter(|j| (*j as usize) < n) {
                    s.joint[j as usize] = true;
                }
            }
        }
        Ok(s)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// World matrices for a pose (one local TRS per node).
    pub fn world(&self, local: &[Trs]) -> Vec<M4> {
        let mut w = vec![M4::ID; self.len()];
        for &i in &self.order {
            let m = local[i].m4();
            w[i] = match self.parent[i] {
                Some(p) => w[p].mul(&m),
                None => m,
            };
        }
        w
    }

    /// World rotations for a pose, from the rotations alone. Scale is left
    /// out on purpose: a rig's 0.01 armature must not bend its bones.
    pub fn world_rot(&self, local: &[Trs]) -> Vec<Q> {
        let mut w = vec![Q::ID; self.len()];
        for &i in &self.order {
            w[i] = match self.parent[i] {
                Some(p) => w[p].mul(local[i].r).norm(),
                None => local[i].r,
            };
        }
        w
    }

    pub fn is_ancestor(&self, a: usize, mut b: usize) -> bool {
        while let Some(p) = self.parent[b] {
            if p == a {
                return true;
            }
            b = p;
        }
        false
    }
}

fn node_trs(nd: &Value) -> Trs {
    let f = |k: &str| -> Option<Vec<f64>> {
        nd.get(k).and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect())
    };
    if let Some(m) = f("matrix").filter(|m| m.len() == 16) {
        let mut a = [0.0; 16];
        a.copy_from_slice(&m);
        let (t, r, s) = M4(a).decompose();
        return Trs { t, r, s };
    }
    Trs {
        t: f("translation").filter(|v| v.len() == 3).map(|v| V3::from(&v)).unwrap_or(V3::ZERO),
        r: f("rotation").filter(|v| v.len() == 4).map(|v| Q::from(&v)).unwrap_or(Q::ID),
        s: f("scale").filter(|v| v.len() == 3).map(|v| V3::from(&v)).unwrap_or(V3(1.0, 1.0, 1.0)),
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Path {
    T,
    R,
    S,
}

pub struct Channel {
    pub node: usize,
    pub path: Path,
    pub times: Vec<f64>,
    /// One value per time (cubic splines keep only their value, not tangents).
    pub values: Vec<Vec<f64>>,
    pub step: bool,
}

pub struct Clip {
    pub name: String,
    pub channels: Vec<Channel>,
    pub duration: f64,
}

impl Clip {
    pub fn read_all(g: &Glb, n_nodes: usize) -> Result<Vec<Clip>, String> {
        let mut out = Vec::new();
        for (ai, a) in g.list("animations").iter().enumerate() {
            let name = a.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("clip{ai}"));
            let samplers = a.get("samplers").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut channels = Vec::new();
            let mut duration: f64 = 0.0;
            for ch in a.get("channels").and_then(Value::as_array).into_iter().flatten() {
                let target = &ch["target"];
                let Some(node) = target.get("node").and_then(Value::as_u64).map(|n| n as usize) else { continue };
                if node >= n_nodes {
                    continue;
                }
                let path = match target.get("path").and_then(Value::as_str) {
                    Some("translation") => Path::T,
                    Some("rotation") => Path::R,
                    Some("scale") => Path::S,
                    _ => continue, // morph weights and pointers are not the rig's
                };
                let Some(sm) = ch.get("sampler").and_then(Value::as_u64).and_then(|i| samplers.get(i as usize)) else { continue };
                let interp = sm.get("interpolation").and_then(Value::as_str).unwrap_or("LINEAR");
                let (times, _) = g.read(sm["input"].as_u64().unwrap_or(u64::MAX) as usize)?;
                let (flat, width) = g.read(sm["output"].as_u64().unwrap_or(u64::MAX) as usize)?;
                let per = if interp == "CUBICSPLINE" { 3 } else { 1 };
                let mut values = Vec::with_capacity(times.len());
                for k in 0..times.len() {
                    let at = (k * per + per / 2) * width;
                    match flat.get(at..at + width) {
                        Some(v) => values.push(v.to_vec()),
                        None => return Err(format!("{name}: a sampler has fewer values than times")),
                    }
                }
                if let Some(last) = times.last() {
                    duration = duration.max(*last);
                }
                channels.push(Channel { node, path, times, values, step: interp == "STEP" });
            }
            out.push(Clip { name, channels, duration });
        }
        Ok(out)
    }

    /// Every key time of the channels on these nodes, sorted, merged within 0.1 ms.
    pub fn key_times(&self, nodes: &[usize]) -> Vec<f64> {
        let mut t: Vec<f64> = self
            .channels
            .iter()
            .filter(|c| nodes.contains(&c.node))
            .flat_map(|c| c.times.iter().copied())
            .collect();
        t.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        t.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        if t.is_empty() {
            t.push(0.0);
        }
        t
    }

    /// The local pose at time `t`: rest, with each channel's value laid over.
    pub fn pose(&self, rest: &[Trs], t: f64) -> Vec<Trs> {
        let mut p = rest.to_vec();
        for c in &self.channels {
            let v = c.sample(t);
            match c.path {
                Path::T => p[c.node].t = V3::from(&v),
                Path::R => p[c.node].r = Q::from(&v),
                Path::S => p[c.node].s = V3::from(&v),
            }
        }
        p
    }
}

impl Channel {
    pub fn sample(&self, t: f64) -> Vec<f64> {
        let n = self.times.len();
        if n == 0 {
            return vec![0.0, 0.0, 0.0, 1.0];
        }
        if t <= self.times[0] || n == 1 {
            return self.values[0].clone();
        }
        if t >= self.times[n - 1] {
            return self.values[n - 1].clone();
        }
        let k = self.times.partition_point(|x| *x <= t).saturating_sub(1).min(n - 2);
        let (t0, t1) = (self.times[k], self.times[k + 1]);
        let f = if self.step || t1 <= t0 { 0.0 } else { (t - t0) / (t1 - t0) };
        let (a, b) = (&self.values[k], &self.values[k + 1]);
        if self.path == Path::R {
            let q = Q::from(a).slerp(Q::from(b), f);
            return vec![q.0, q.1, q.2, q.3];
        }
        a.iter().zip(b).map(|(x, y)| x + (y - x) * f).collect()
    }
}
