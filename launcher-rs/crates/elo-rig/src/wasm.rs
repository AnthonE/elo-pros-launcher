//! The browser's door: a plain C ABI over linear memory, so the page needs no
//! generated glue (`watchtower/js/animate.js` is the whole caller).
//!
//! The page asks for room with `rig_alloc`, copies a file in, and calls one
//! verb with (pointer, length) pairs; an options pair is UTF-8 JSON. A verb
//! returns 0 and leaves its answer in the output buffer (`rig_out_ptr`,
//! `rig_out_len`), or returns 1 and leaves an error sentence there. A verb
//! that also reports (`rig_retarget`) leaves JSON in the report buffer.

use serde_json::{json, Value};
use std::cell::RefCell;

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static REPORT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[no_mangle]
pub extern "C" fn rig_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` are exactly what `rig_alloc` handed out.
#[no_mangle]
pub unsafe extern "C" fn rig_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

#[no_mangle]
pub extern "C" fn rig_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn rig_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

#[no_mangle]
pub extern "C" fn rig_report_ptr() -> *const u8 {
    REPORT.with(|o| o.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn rig_report_len() -> usize {
    REPORT.with(|o| o.borrow().len())
}

unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

unsafe fn options(ptr: *const u8, len: usize) -> Value {
    serde_json::from_slice(bytes(ptr, len)).unwrap_or(json!({}))
}

fn answer(r: Result<Vec<u8>, String>) -> u32 {
    let (code, body) = match r {
        Ok(b) => (0, b),
        Err(e) => (1, e.into_bytes()),
    };
    OUT.with(|o| *o.borrow_mut() = body);
    code
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

/// # Safety
/// The pair is a live allocation from `rig_alloc`.
#[no_mangle]
pub unsafe extern "C" fn rig_inspect(p: *const u8, n: usize) -> u32 {
    answer(crate::inspect(bytes(p, n)).map(|v| v.to_string().into_bytes()))
}

/// Options: `{"clips": [names], "in_place": bool}`.
///
/// # Safety
/// Each pair is a live allocation from `rig_alloc`.
#[no_mangle]
pub unsafe extern "C" fn rig_retarget(tp: *const u8, tn: usize, sp: *const u8, sn: usize, op: *const u8, on: usize) -> u32 {
    let o = options(op, on);
    let r = crate::retarget(bytes(tp, tn), bytes(sp, sn), &names(&o["clips"]), o["in_place"].as_bool().unwrap_or(false));
    answer(r.map(|(glb, report)| {
        REPORT.with(|x| *x.borrow_mut() = report.to_string().into_bytes());
        glb
    }))
}

/// Options: `{"clip": name, "in_place": bool}`.
///
/// # Safety
/// Each pair is a live allocation from `rig_alloc`.
#[no_mangle]
pub unsafe extern "C" fn rig_vrma(sp: *const u8, sn: usize, op: *const u8, on: usize) -> u32 {
    let o = options(op, on);
    answer(crate::vrma(bytes(sp, sn), o["clip"].as_str().unwrap_or(""), o["in_place"].as_bool().unwrap_or(false)))
}

/// Options: `{"clips": [names]}`.
///
/// # Safety
/// Each pair is a live allocation from `rig_alloc`.
#[no_mangle]
pub unsafe extern "C" fn rig_clips_only(sp: *const u8, sn: usize, op: *const u8, on: usize) -> u32 {
    let o = options(op, on);
    answer(crate::clips_only(bytes(sp, sn), &names(&o["clips"])))
}

/// Options: `{"triangles": n, "name": "…"}`.
///
/// # Safety
/// Each pair is a live allocation from `rig_alloc`.
#[no_mangle]
pub unsafe extern "C" fn rig_readme(sp: *const u8, sn: usize, op: *const u8, on: usize) -> u32 {
    let o = options(op, on);
    let n = o["triangles"].as_u64().unwrap_or(1500) as usize;
    answer(crate::readme_stl(bytes(sp, sn), n, o["name"].as_str().unwrap_or("model")).map(String::into_bytes))
}
