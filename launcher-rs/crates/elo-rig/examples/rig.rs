//! The rig desk from a terminal, for trying files by hand:
//!
//!   cargo run -p elo-rig --example rig -- inspect model.glb
//!   cargo run -p elo-rig --example rig -- retarget model.glb clips.glb out.glb [clip …]
//!   cargo run -p elo-rig --example rig -- vrma clips.glb Walk out.vrma
//!   cargo run -p elo-rig --example rig -- clips clips.glb out.glb [clip …]
//!   cargo run -p elo-rig --example rig -- stl model.glb out.md [triangles]

use std::fs;

fn main() -> Result<(), String> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let read = |p: &str| fs::read(p).map_err(|e| format!("{p}: {e}"));
    let write = |p: &str, b: &[u8]| fs::write(p, b).map_err(|e| format!("{p}: {e}"));
    match a.first().map(String::as_str) {
        Some("inspect") => println!("{:#}", elo_rig::inspect(&read(&a[1])?)?),
        Some("retarget") => {
            let (out, report) = elo_rig::retarget(&read(&a[1])?, &read(&a[2])?, &a[4..], false)?;
            write(&a[3], &out)?;
            println!("{report:#}");
        }
        Some("vrma") => write(&a[3], &elo_rig::vrma(&read(&a[1])?, &a[2], false)?)?,
        Some("clips") => write(&a[2], &elo_rig::clips_only(&read(&a[1])?, &a[3..])?)?,
        Some("stl") => {
            let n = a.get(3).and_then(|n| n.parse().ok()).unwrap_or(1500);
            write(&a[2], elo_rig::readme_stl(&read(&a[1])?, n, "model")?.as_bytes())?
        }
        _ => return Err("usage: rig inspect|retarget|vrma|clips|stl …".into()),
    }
    Ok(())
}
