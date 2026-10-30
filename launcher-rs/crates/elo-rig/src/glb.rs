//! A GLB in and a GLB out: the JSON kept as a `Value`, so every extension a
//! file carries (VRM's among them) passes through untouched, and the binary
//! chunk only ever grows at its end.

use serde_json::{json, Value};

const MAGIC: u32 = 0x4654_6C67; // "glTF"
const JSON_CHUNK: u32 = 0x4E4F_534A;
const BIN_CHUNK: u32 = 0x004E_4942;

pub struct Glb {
    pub json: Value,
    pub bin: Vec<u8>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn num(v: &Value, key: &str, default: u64) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(default)
}

impl Glb {
    pub fn parse(bytes: &[u8]) -> Result<Glb, String> {
        if bytes.first() == Some(&b'{') {
            return Err("this is a .gltf text file; save it as a binary .glb (or .vrm) and try again".into());
        }
        if u32_at(bytes, 0) != Some(MAGIC) {
            return Err("not a GLB, VRM or VRMA file (no glTF header)".into());
        }
        if u32_at(bytes, 4) != Some(2) {
            return Err("only glTF 2.0 files are read".into());
        }
        let total = (u32_at(bytes, 8).unwrap_or(0) as usize).min(bytes.len());
        let mut at = 12;
        let (mut json, mut bin) = (None, Vec::new());
        while at + 8 <= total {
            let len = u32_at(bytes, at).unwrap_or(0) as usize;
            let kind = u32_at(bytes, at + 4).unwrap_or(0);
            let body = bytes.get(at + 8..at + 8 + len).ok_or("the file is cut short")?;
            match kind {
                JSON_CHUNK if json.is_none() => {
                    json = Some(serde_json::from_slice::<Value>(body).map_err(|e| format!("the glTF JSON does not parse: {e}"))?);
                }
                BIN_CHUNK if bin.is_empty() => bin = body.to_vec(),
                _ => {}
            }
            at += 8 + len;
        }
        let json = json.ok_or("the file has no JSON chunk")?;
        let glb = Glb { json, bin };
        for (i, b) in arr(&glb.json, "buffers").iter().enumerate() {
            if b.get("uri").is_some() {
                return Err(format!(
                    "buffer {i} lives in a separate file; export one self-contained .glb"
                ));
            }
        }
        Ok(glb)
    }

    pub fn encode(&mut self) -> Vec<u8> {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        if !self.bin.is_empty() {
            let buffers = self.json.as_object_mut().unwrap().entry("buffers").or_insert_with(|| json!([]));
            if buffers.as_array().is_none_or(|a| a.is_empty()) {
                *buffers = json!([{}]);
            }
            buffers[0]["byteLength"] = json!(self.bin.len());
        }
        let mut text = serde_json::to_vec(&self.json).expect("a Value always serialises");
        while text.len() % 4 != 0 {
            text.push(b' ');
        }
        let bin_part = if self.bin.is_empty() { 0 } else { 8 + self.bin.len() };
        let total = 12 + 8 + text.len() + bin_part;
        let mut out = Vec::with_capacity(total);
        out.extend(MAGIC.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.extend((total as u32).to_le_bytes());
        out.extend((text.len() as u32).to_le_bytes());
        out.extend(JSON_CHUNK.to_le_bytes());
        out.extend(&text);
        if !self.bin.is_empty() {
            out.extend((self.bin.len() as u32).to_le_bytes());
            out.extend(BIN_CHUNK.to_le_bytes());
            out.extend(&self.bin);
        }
        out
    }

    pub fn list(&self, key: &str) -> &[Value] {
        arr(&self.json, key)
    }

    /// An accessor's elements, flattened, as f64. Normalised integers come back
    /// in -1..1 or 0..1; others as their integer value.
    pub fn read(&self, accessor: usize) -> Result<(Vec<f64>, usize), String> {
        let acc = self.list("accessors").get(accessor).ok_or(format!("no accessor {accessor}"))?;
        let width = match acc.get("type").and_then(Value::as_str) {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            Some("MAT4") => 16,
            other => return Err(format!("accessor {accessor} has type {other:?}, which is not read here")),
        };
        let count = num(acc, "count", 0) as usize;
        let ct = num(acc, "componentType", 5126);
        let normalized = acc.get("normalized").and_then(Value::as_bool).unwrap_or(false);
        let size = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(format!("accessor {accessor} has component type {ct}")),
        };
        let mut out = vec![0.0; count * width];
        if let Some(view) = acc.get("bufferView").and_then(Value::as_u64) {
            let bv = self.list("bufferViews").get(view as usize).ok_or(format!("no bufferView {view}"))?;
            if num(bv, "buffer", 0) != 0 {
                return Err("only the GLB's own binary chunk is read".into());
            }
            let base = (num(bv, "byteOffset", 0) + num(acc, "byteOffset", 0)) as usize;
            let stride = match num(bv, "byteStride", 0) as usize {
                0 => size * width,
                s => s,
            };
            let end = base + stride * count.saturating_sub(1) + size * width;
            if count > 0 && end > self.bin.len() {
                return Err(format!("accessor {accessor} runs past the end of the binary chunk"));
            }
            for i in 0..count {
                for c in 0..width {
                    out[i * width + c] = self.component(base + i * stride + c * size, ct, normalized);
                }
            }
        }
        if let Some(sparse) = acc.get("sparse") {
            let n = num(sparse, "count", 0) as usize;
            let idx = &sparse["indices"];
            let val = &sparse["values"];
            let view_at = |v: &Value| -> Result<usize, String> {
                let bv = self
                    .list("bufferViews")
                    .get(num(v, "bufferView", u64::MAX) as usize)
                    .ok_or("a sparse accessor names no bufferView")?;
                Ok((num(bv, "byteOffset", 0) + num(v, "byteOffset", 0)) as usize)
            };
            let (ia, va) = (view_at(idx)?, view_at(val)?);
            let ict = num(idx, "componentType", 5125);
            let isz = if ict == 5121 { 1 } else if ict == 5123 { 2 } else { 4 };
            for k in 0..n {
                let at = ia + k * isz;
                if at + isz > self.bin.len() || va + (k + 1) * width * size > self.bin.len() {
                    return Err("a sparse accessor runs past the binary chunk".into());
                }
                let i = self.component(at, ict, false) as usize;
                if i >= count {
                    continue;
                }
                for c in 0..width {
                    out[i * width + c] = self.component(va + (k * width + c) * size, ct, normalized);
                }
            }
        }
        Ok((out, width))
    }

    fn component(&self, at: usize, ct: u64, normalized: bool) -> f64 {
        let b = &self.bin;
        match ct {
            5120 => {
                let v = b[at] as i8 as f64;
                if normalized { (v / 127.0).max(-1.0) } else { v }
            }
            5121 => {
                let v = b[at] as f64;
                if normalized { v / 255.0 } else { v }
            }
            5122 => {
                let v = i16::from_le_bytes([b[at], b[at + 1]]) as f64;
                if normalized { (v / 32767.0).max(-1.0) } else { v }
            }
            5123 => {
                let v = u16::from_le_bytes([b[at], b[at + 1]]) as f64;
                if normalized { v / 65535.0 } else { v }
            }
            5125 => u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as f64,
            _ => f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as f64,
        }
    }

    /// Append f32 data as a new bufferView + accessor; returns the accessor.
    /// `minmax` writes the bounds glTF requires on animation inputs.
    pub fn push_f32(&mut self, data: &[f32], kind: &str, minmax: bool) -> usize {
        let width = match kind {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            _ => 4,
        };
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let offset = self.bin.len();
        for v in data {
            self.bin.extend(v.to_le_bytes());
        }
        let root = self.json.as_object_mut().expect("glTF JSON is an object");
        if root.get("buffers").and_then(Value::as_array).is_none_or(|a| a.is_empty()) {
            root.insert("buffers".into(), json!([{ "byteLength": 0 }]));
        }
        let views = root.entry("bufferViews").or_insert_with(|| json!([]));
        let views = views.as_array_mut().expect("bufferViews is an array");
        views.push(json!({ "buffer": 0, "byteOffset": offset, "byteLength": data.len() * 4 }));
        let view = views.len() - 1;
        let mut acc = json!({
            "bufferView": view, "componentType": 5126, "count": data.len() / width, "type": kind,
        });
        if minmax && !data.is_empty() {
            let (mut lo, mut hi) = (vec![f32::MAX; width], vec![f32::MIN; width]);
            for (i, v) in data.iter().enumerate() {
                lo[i % width] = lo[i % width].min(*v);
                hi[i % width] = hi[i % width].max(*v);
            }
            acc["min"] = json!(lo);
            acc["max"] = json!(hi);
        }
        let accs = root.entry("accessors").or_insert_with(|| json!([]));
        let accs = accs.as_array_mut().expect("accessors is an array");
        accs.push(acc);
        accs.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_glb_reads_back() {
        let mut g = Glb { json: json!({ "asset": { "version": "2.0" } }), bin: vec![] };
        let a = g.push_f32(&[0.0, 0.5, 1.0], "SCALAR", true);
        let b = g.push_f32(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], "VEC3", false);
        let bytes = g.encode();
        assert_eq!(bytes.len() % 4, 0);
        let back = Glb::parse(&bytes).unwrap();
        assert_eq!(back.read(a).unwrap(), (vec![0.0, 0.5, 1.0], 1));
        assert_eq!(back.read(b).unwrap().0, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(back.json["accessors"][a]["max"], json!([1.0]));
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        assert!(Glb::parse(b"{\"asset\":{}}").err().unwrap().contains(".glb"));
        assert!(Glb::parse(b"PK\x03\x04").is_err());
    }
}
