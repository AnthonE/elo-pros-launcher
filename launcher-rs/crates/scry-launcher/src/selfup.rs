//! `scry self-update` — the client replacing the client.
//!
//! Operator, 2026-08-15: *"this downloader needs to update itself or have an
//! updater it can run."* That sentence reverses a recorded decision
//! (`docs/LAUNCHER.md` §12b said *"a notice, not a self-update"*), so the
//! reasons the old ruling gave are answered here one by one rather than
//! quietly dropped:
//!
//! * **"it needs its own signature policy"** — the artifact is verified
//!   against the sha256 the origin publishes on the `/api/launcher` card,
//!   which is read from the same `SHA256SUMS` the download page tells a human
//!   to check. Nothing is staged, let alone run, before the hash matches.
//!   That is the launcher's existing trust model — the depot digest applied
//!   to ourselves — not a new one.
//! * **"its own rollback"** — the previous binary stays beside the new one as
//!   `scry.old` / `scry-gui.old` until the *next* self-update replaces it.
//!   Rolling back is one rename, and the report says so.
//! * **"its own answer for dying mid-write"** — the new binary is written
//!   whole to `<name>.new` in the same directory and fsynced, smoke-run, and
//!   only then renamed in. On unix the rename is atomic over the old name and
//!   the rollback copy is hardlinked first, so there is no instant with no
//!   binary at the target path. (Windows cannot rename onto an occupied name,
//!   so it gets the two renames and a stated, one-line-wide gap.)
//! * **"the package manager owns this on Linux"** — where it really does, it
//!   still does: an install under `/usr/bin` is dpkg's, so this downloads and
//!   verifies the `.deb` and hands the one privileged command to the holder
//!   instead of scribbling over files `dpkg -V` would then flag. Exit **12**
//!   marks that half-done state so a script can tell it from "updated".
//!
//! And the wall the old ruling was defending still stands: **"we could not
//! look" is never rendered as "you are up to date"** — an unreachable origin
//! exits 3 with the reason, the same code `scry status` uses.
//!
//! The GUI does not link any of this. `scry-gui` is the window; the updater it
//! can run is `scry self-update`, which is the second half of the operator's
//! sentence and the reason the CLI exists first.

use crate::prompt;
use scry_depot::install::human;
use scry_depot::update::version_cmp;
use scry_depot::DepotError;
use scry_net::Net;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};

/// A release artifact is a couple of stripped binaries in a compressed
/// container — single-digit megabytes today. Sixty-four is ten times that;
/// past it, this is not our artifact.
pub const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
/// One extracted binary. The decompression bound, so a hostile document
/// cannot balloon in memory — the sha256 gate in front of this makes that a
/// second wall rather than the first.
const MAX_MEMBER_BYTES: usize = 256 * 1024 * 1024;

// ── the card ─────────────────────────────────────────────────────────────────

/// One downloadable row off the origin's `/api/launcher` card.
#[derive(Debug, Clone)]
pub struct Row {
    /// The artifact's filename — `scry-0.5.0-linux-x86_64.tar.gz` and kin,
    /// exactly as `build_release.py` names them. Matching happens on this.
    pub name: String,
    /// Where to fetch it, already completed against the host we asked.
    pub url: String,
    pub sha256: Option<String>,
    pub bytes: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Card {
    pub version: Option<String>,
    pub rows: Vec<Row>,
}

/// Read the card **liberally in the keys, strictly in the values.**
///
/// The card's prose contract is documented (`docs/LAUNCHER.md` §12b: `get_it`
/// rows, each carrying a real filename and a sha256 measured off the published
/// `SHA256SUMS`), but the exact key names live in `meter/launcher.py`, which
/// this repo does not carry. So rows are found under `get_it` (array or
/// object), the url under any of `url`/`path`/`href`, the name under
/// `name`/`file`/`filename`/`artifact` or the url's last segment, and the hash
/// under any key containing `sha` whose value is 64 hex characters. What is
/// *not* liberal: a row with no resolvable url is dropped, a protocol-relative
/// `//host/…` url is dropped for the same reason `Manifest::rebase` blanks one
/// (it looks relative and is not), and a missing sha256 survives parsing only
/// to be refused at install time by name.
pub fn parse_card(v: &Value, host: &str) -> Card {
    let version = v
        .get("client")
        .and_then(|c| c.get("version"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut rows = Vec::new();
    let found = v
        .get("get_it")
        .or_else(|| v.get("client").and_then(|c| c.get("get_it")));
    let objects: Vec<&Value> = match found {
        Some(Value::Array(a)) => a.iter().collect(),
        // An object keyed by kind reads the same as an array of its values.
        Some(Value::Object(m)) => m.values().collect(),
        _ => Vec::new(),
    };
    for o in objects {
        let Some(m) = o.as_object() else { continue };
        let url_raw = ["url", "path", "href"]
            .iter()
            .find_map(|k| m.get(*k).and_then(Value::as_str))
            .unwrap_or("");
        let Some(url) = complete_url(url_raw, host) else {
            continue;
        };
        let name = ["name", "file", "filename", "artifact"]
            .iter()
            .find_map(|k| m.get(*k).and_then(Value::as_str))
            .map(str::to_string)
            .unwrap_or_else(|| basename(&url));
        // Any key that says sha, one value shape: 64 hex. `sha256`,
        // `sha_256`, `sha-256` all land here without a list to maintain.
        let sha256 = m.iter().find_map(|(k, val)| {
            if !k.to_ascii_lowercase().contains("sha") {
                return None;
            }
            val.as_str()
                .map(str::trim)
                .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_ascii_lowercase)
        });
        let bytes = ["bytes", "size"]
            .iter()
            .find_map(|k| m.get(*k).and_then(Value::as_u64));
        rows.push(Row {
            name,
            url,
            sha256,
            bytes,
        });
    }
    Card { version, rows }
}

/// The client completes relative urls; the server does not write absolute
/// ones. Same rule, same reason and same protocol-relative refusal as
/// `Manifest::rebase`.
fn complete_url(raw: &str, host: &str) -> Option<String> {
    if raw.is_empty() || raw.starts_with("//") {
        return None;
    }
    if raw.starts_with("https://") || raw.starts_with("http://") {
        return Some(raw.to_string());
    }
    Some(format!(
        "{}/{}",
        host.trim_end_matches('/'),
        raw.trim_start_matches('/')
    ))
}

fn basename(url: &str) -> String {
    url.rsplit('/').next().unwrap_or(url).to_string()
}

// ── which artifact is ours ───────────────────────────────────────────────────

/// How the bytes will be opened once they arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    TarGz,
    Zip,
    /// Not opened at all — verified and handed to the package manager.
    Deb,
}

/// The artifact for this machine, matched on the **filename** — the names are
/// written by `launcher-rs/build_release.py` in this same repository, so the
/// pattern and the artifact have one source. A card that lists artifacts but
/// none for this platform is a real answer and the error names both sides.
pub fn pick<'a>(rows: &'a [Row], platform: &str, dpkg: bool) -> Result<(&'a Row, Container), String> {
    let want: (&dyn Fn(&str) -> bool, Container, String) = match platform {
        "linux-x86_64" if dpkg => (
            &|n: &str| n.starts_with("scry_") && n.ends_with("_amd64.deb"),
            Container::Deb,
            "the .deb".into(),
        ),
        "linux-x86_64" => (
            &|n: &str| n.starts_with("scry-") && n.ends_with("-linux-x86_64.tar.gz"),
            Container::TarGz,
            "the linux tarball".into(),
        ),
        "win-x86_64" => (
            &|n: &str| n.starts_with("scry-") && n.ends_with("-windows-x86_64.zip"),
            Container::Zip,
            "the windows zip".into(),
        ),
        "mac-arm64" | "mac-x86_64" => (
            &|n: &str| n.starts_with("scry-") && n.ends_with("-macos-universal.tar.gz"),
            Container::TarGz,
            "the macOS tarball".into(),
        ),
        other => {
            return Err(format!(
                "no artifact is published for {other} — the card lists {}",
                listed(rows)
            ))
        }
    };
    match rows.iter().find(|r| want.0(&r.name)) {
        Some(r) => Ok((r, want.1)),
        None => Err(format!(
            "the origin's card lists no artifact for {platform} ({}) — it has {}",
            want.2,
            listed(rows)
        )),
    }
}

fn listed(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "no downloads at all".into();
    }
    rows.iter()
        .map(|r| r.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

// ── opening the containers ───────────────────────────────────────────────────
//
// These parse input whose sha256 has ALREADY matched what the origin
// published, so they are the second wall and are allowed to be strict and
// small: refuse what is not understood rather than recover. Container
// checksums (gzip CRC32, tar header sums, zip CRC32) are deliberately not
// re-verified — sha256 over the whole artifact already pinned every byte of
// them, and a second, weaker checksum can only disagree by a bug here.

/// The DEFLATE payload out of a gzip envelope (RFC 1952), bounded.
pub fn gunzip(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if data.len() < 18 {
        return Err("too short to be a gzip file".into());
    }
    if data[0] != 0x1f || data[1] != 0x8b {
        return Err("not a gzip file (bad magic)".into());
    }
    if data[2] != 8 {
        return Err(format!("gzip method {} is not DEFLATE", data[2]));
    }
    let flags = data[3];
    let mut at = 10usize;
    if flags & 0x04 != 0 {
        // FEXTRA
        if at + 2 > data.len() {
            return Err("gzip FEXTRA is truncated".into());
        }
        let n = u16::from_le_bytes([data[at], data[at + 1]]) as usize;
        at += 2 + n;
    }
    for bit in [0x08u8, 0x10] {
        // FNAME, then FCOMMENT: NUL-terminated strings.
        if flags & bit != 0 {
            match data[at.min(data.len())..].iter().position(|b| *b == 0) {
                Some(p) => at += p + 1,
                None => return Err("gzip name/comment is unterminated".into()),
            }
        }
    }
    if flags & 0x02 != 0 {
        at += 2; // FHCRC
    }
    if at + 8 > data.len() {
        return Err("gzip payload is missing".into());
    }
    // The last 8 bytes are CRC32 + ISIZE. See the module note on checksums.
    miniz_oxide::inflate::decompress_to_vec_with_limit(&data[at..data.len() - 8], limit)
        .map_err(|e| format!("could not inflate: {e:?}"))
}

/// Every regular file in a tar, as `(path, bytes)`.
///
/// Reads the shapes our own packager writes (ustar/GNU, plus the `L` longname
/// record Python emits for paths over 100 bytes) and skips the rest by size,
/// so an unknown member type cannot desynchronise the walk.
pub fn tar_members(tar: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut longname: Option<String> = None;
    while at + 512 <= tar.len() {
        let block = &tar[at..at + 512];
        at += 512;
        if block.iter().all(|b| *b == 0) {
            break; // end-of-archive marker
        }
        let size = octal(&block[124..136])
            .ok_or_else(|| format!("unreadable size in tar header at {}", at - 512))?;
        let data_end = at
            .checked_add(size)
            .filter(|e| *e <= tar.len())
            .ok_or("tar member runs past the end of the archive")?;
        let data = &tar[at..data_end];
        at = data_end + (512 - size % 512) % 512; // members are 512-padded
        let typeflag = block[156];
        match typeflag {
            b'L' => {
                // GNU: this member's DATA is the next member's name.
                longname = Some(cstr(data).to_string());
            }
            b'0' | 0 => {
                let name = longname.take().unwrap_or_else(|| {
                    let mut n = cstr(&block[0..100]).to_string();
                    // ustar splits long paths into prefix + name.
                    if &block[257..262] == b"ustar" {
                        let prefix = cstr(&block[345..500]);
                        if !prefix.is_empty() {
                            n = format!("{prefix}/{n}");
                        }
                    }
                    n
                });
                out.push((name, data.to_vec()));
            }
            // Directories, links, pax headers: their bytes were skipped above,
            // which is all "ignored" has to mean.
            _ => {
                longname = None;
            }
        }
    }
    Ok(out)
}

fn octal(field: &[u8]) -> Option<usize> {
    let s = std::str::from_utf8(field).ok()?;
    let s = s.trim_matches(|c: char| c == '\0' || c == ' ');
    if s.is_empty() {
        return Some(0);
    }
    usize::from_str_radix(s, 8).ok()
}

fn cstr(field: &[u8]) -> &str {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..end]).unwrap_or("")
}

/// Every file in a zip, as `(path, bytes)`. Stored and DEFLATE entries only —
/// which is every entry our packager writes — and no zip64, because a client
/// artifact past 4 GB is not a client artifact.
pub fn zip_members(zip: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    // The end-of-central-directory record is within the last 64 KiB + 22
    // bytes; scan back for its signature.
    let tail_start = zip.len().saturating_sub(65_558);
    let eocd = (tail_start..zip.len().saturating_sub(21))
        .rev()
        .find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or("no zip end-of-central-directory record")?;
    let count = u16::from_le_bytes([zip[eocd + 10], zip[eocd + 11]]) as usize;
    let cd_at = le32(zip, eocd + 16)? as usize;

    let mut out = Vec::new();
    let mut at = cd_at;
    for _ in 0..count {
        if at + 46 > zip.len() || zip[at..at + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err("zip central directory is malformed".into());
        }
        let method = u16::from_le_bytes([zip[at + 10], zip[at + 11]]);
        let comp = le32(zip, at + 20)? as usize;
        let uncomp = le32(zip, at + 24)? as usize;
        let name_len = u16::from_le_bytes([zip[at + 28], zip[at + 29]]) as usize;
        let extra_len = u16::from_le_bytes([zip[at + 30], zip[at + 31]]) as usize;
        let comment_len = u16::from_le_bytes([zip[at + 32], zip[at + 33]]) as usize;
        let local_at = le32(zip, at + 42)? as usize;
        if comp == 0xffff_ffff || uncomp == 0xffff_ffff || local_at == 0xffff_ffff {
            return Err("zip64 is not a shape a client artifact takes".into());
        }
        let name = std::str::from_utf8(
            zip.get(at + 46..at + 46 + name_len)
                .ok_or("zip name runs past the end")?,
        )
        .map_err(|_| "zip member name is not utf-8")?
        .to_string();
        at += 46 + name_len + extra_len + comment_len;

        // The local header repeats the name/extra lengths and they may
        // differ from the central copy; the data sits after the LOCAL ones.
        if local_at + 30 > zip.len() || zip[local_at..local_at + 4] != [0x50, 0x4b, 0x03, 0x04] {
            return Err(format!("zip local header for {name} is malformed"));
        }
        let lnl = u16::from_le_bytes([zip[local_at + 26], zip[local_at + 27]]) as usize;
        let lel = u16::from_le_bytes([zip[local_at + 28], zip[local_at + 29]]) as usize;
        let data_at = local_at + 30 + lnl + lel;
        let raw = zip
            .get(data_at..data_at + comp)
            .ok_or_else(|| format!("zip member {name} runs past the end"))?;
        if uncomp > MAX_MEMBER_BYTES {
            return Err(format!("zip member {name} is over the size bound"));
        }
        let bytes = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, MAX_MEMBER_BYTES)
                .map_err(|e| format!("could not inflate {name}: {e:?}"))?,
            m => return Err(format!("zip member {name} uses method {m}, not stored/DEFLATE")),
        };
        if bytes.len() != uncomp {
            return Err(format!(
                "zip member {name} inflated to {} bytes, the directory said {uncomp}",
                bytes.len()
            ));
        }
        out.push((name, bytes));
    }
    Ok(out)
}

fn le32(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "zip record runs past the end".into())
}

pub fn sha_hex(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The named binary out of an archive, matched on the trailing path
/// component so the versioned root directory never matters.
fn member<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
    files
        .iter()
        .find(|(p, _)| p.rsplit('/').next() == Some(name))
        .map(|(_, b)| b.as_slice())
}

// ── the install being replaced ───────────────────────────────────────────────

/// Where this client lives and what that means for how it updates.
#[derive(Debug)]
pub enum Shape {
    /// A directory we can write: the tarball/zip install. Replaced in place.
    InPlace { dir: PathBuf },
    /// dpkg's territory. Verified download, then the one privileged command
    /// is the holder's to run — scribbling over `/usr/bin` ourselves would
    /// make `dpkg -V scry` report the package tampered with, which it would
    /// then be right about.
    Dpkg { dir: PathBuf },
}

/// `SCRY_SELF_DIR` overrides where "this client" is, which is the test seam
/// and nothing else worth advertising: it grants nothing the caller's own
/// file permissions did not already grant.
pub fn install_shape() -> Result<Shape, DepotError> {
    let dir = match std::env::var("SCRY_SELF_DIR") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => {
            let exe = std::env::current_exe()
                .map_err(|e| DepotError::new(format!("cannot locate this binary: {e}")))?;
            exe.parent()
                .ok_or_else(|| DepotError::new("this binary has no parent directory"))?
                .to_path_buf()
        }
    };
    let dir = dir
        .canonicalize()
        .map_err(|e| DepotError::new(format!("{}: {e}", dir.display())))?;
    if cfg!(target_os = "linux") && (dir == Path::new("/usr/bin") || dir == Path::new("/usr/games"))
    {
        return Ok(Shape::Dpkg { dir });
    }
    Ok(Shape::InPlace { dir })
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

// ── the swap ─────────────────────────────────────────────────────────────────

/// Write the new binary whole, beside its target, executable, synced.
pub fn stage(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write as _;
    let staged = dir.join(format!("{name}.new"));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o755);
    }
    let mut f = opts.open(&staged).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "{} is not writable by you — re-run with the privileges that \
                 own it, or move the install somewhere you own",
                dir.display()
            )
        } else {
            format!("{}: {e}", staged.display())
        }
    })?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| format!("{}: {e}", staged.display()))?;
    #[cfg(unix)]
    {
        // The file may have existed from a dead earlier run, in which case
        // `mode(0o755)` above did not apply — it sets creation mode only.
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", staged.display()))?;
    }
    Ok(staged)
}

/// Prove the staged binary at least loads and runs on this machine — the one
/// property no hash can carry. `help` prints usage and touches no network.
pub fn smoke(staged: &Path) -> Result<(), String> {
    let out = std::process::Command::new(staged)
        .arg("help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("the staged binary would not start: {e}"))?;
    if !out.success() {
        return Err(format!(
            "the staged binary started and failed its smoke run ({out}) — \
             nothing was replaced"
        ));
    }
    Ok(())
}

/// Land `<name>.new` on `<name>`, keeping the previous binary as
/// `<name>.old`.
///
/// The unix order is: hardlink the current binary to `.old` (the target never
/// vanishes), then one atomic rename over it — a crash at any point leaves a
/// runnable client. Windows cannot rename onto an occupied name and cannot
/// delete a running image, so it gets rename-away then rename-in, with a
/// rollback rename if the second half fails; the `.old` is the same rollback
/// there, it just cannot be made gap-free.
pub fn commit(dir: &Path, name: &str) -> Result<(), String> {
    let target = dir.join(name);
    let staged = dir.join(format!("{name}.new"));
    let old = dir.join(format!("{name}.old"));
    let _ = std::fs::remove_file(&old);
    #[cfg(unix)]
    {
        if target.exists() {
            std::fs::hard_link(&target, &old)
                .map_err(|e| format!("could not keep a rollback copy at {}: {e}", old.display()))?;
        }
        std::fs::rename(&staged, &target).map_err(|e| format!("{}: {e}", target.display()))
    }
    #[cfg(not(unix))]
    {
        if target.exists() {
            std::fs::rename(&target, &old)
                .map_err(|e| format!("could not set the old binary aside: {e}"))?;
        }
        std::fs::rename(&staged, &target).map_err(|e| {
            let rolled = std::fs::rename(&old, &target).is_ok();
            format!(
                "{}: {e}{}",
                target.display(),
                if rolled {
                    " — the previous binary was put back"
                } else {
                    " — the previous binary is beside it as .old; rename it back"
                }
            )
        })
    }
}

// ── the verb ─────────────────────────────────────────────────────────────────

/// What `scry self-update` says with `--json`, and the exit codes scripts
/// read: 0 done or nothing to do, 1 refused or failed, **3 could not look**,
/// **12 verified but a privileged step remains** (the printed command).
fn say(json: bool, code: i32, state: &str, detail: Value) -> Result<i32, DepotError> {
    if json {
        let mut v = serde_json::json!({ "state": state });
        if let (Some(m), Some(d)) = (v.as_object_mut(), detail.as_object()) {
            for (k, val) in d {
                m.insert(k.clone(), val.clone());
            }
        }
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    Ok(code)
}

pub fn run(
    host: &str,
    platform: &str,
    state_root: &Path,
    yes: bool,
    dry: bool,
    json: bool,
    insecure: bool,
) -> Result<i32, DepotError> {
    let mine = env!("CARGO_PKG_VERSION");
    let net = Net::new();
    let got = net.get_json(&format!("{host}/api/launcher"));
    let Some(v) = got.value else {
        // Never "up to date". The one sentence this verb must not blur.
        if !json {
            println!(
                "could not check for a newer client — {}",
                if got.reachable {
                    got.why.clone()
                } else {
                    format!("could not reach {host} — {}", got.why)
                }
            );
        }
        return say(json, 3, "unknown", serde_json::json!({ "why": got.why }));
    };
    let card = parse_card(&v, host);
    let Some(there) = card.version.clone() else {
        if !json {
            println!("{host} publishes no client — nothing to take");
        }
        return say(json, 0, "no-client", serde_json::json!({}));
    };
    match version_cmp(&there, mine) {
        std::cmp::Ordering::Equal => {
            if !json {
                println!("{mine} — current");
            }
            return say(json, 0, "current", serde_json::json!({ "version": mine }));
        }
        std::cmp::Ordering::Less => {
            if !json {
                println!("{mine} — ahead of the origin's {there}; nothing to take");
            }
            return say(
                json,
                0,
                "ahead",
                serde_json::json!({ "mine": mine, "published": there }),
            );
        }
        std::cmp::Ordering::Greater => {}
    }

    let shape = install_shape()?;
    let dpkg = matches!(shape, Shape::Dpkg { .. });
    let (row, container) =
        pick(&card.rows, platform, dpkg).map_err(DepotError::new)?;
    if !row.url.starts_with("https://") && !(insecure && row.url.starts_with("http://")) {
        return Err(DepotError::new(format!(
            "refusing a non-https artifact url ({}) — --allow-insecure permits \
             http for a local test origin",
            row.url
        )));
    }
    let Some(want_hash) = row.sha256.clone() else {
        // The hash is the whole authority to install. No hash, no bytes.
        return Err(DepotError::new(format!(
            "the card offers no sha256 for {} — refusing to install what \
             cannot be checked",
            row.name
        )));
    };

    let dir = match &shape {
        Shape::InPlace { dir } | Shape::Dpkg { dir } => dir.clone(),
    };
    if !json {
        println!("client {mine} → {there}");
        println!(
            "  artifact  {}{}",
            row.name,
            row.bytes.map(|b| format!("  ({})", human(b as i64))).unwrap_or_default()
        );
        println!("  sha256    {want_hash}");
        println!(
            "  replaces  {}",
            if dpkg {
                format!("{} (dpkg's — apt finishes it)", dir.display())
            } else {
                dir.display().to_string()
            }
        );
    }
    if dry {
        return say(
            json,
            0,
            "dry-run",
            serde_json::json!({ "from": mine, "to": there, "artifact": row.name }),
        );
    }
    if !yes && !prompt::confirm("take it? [y/N] ").unwrap_or(false) {
        if !json {
            println!("kept {mine} — nothing changed");
        }
        return say(json, 0, "kept", serde_json::json!({}));
    }

    // Learn "you cannot write here" for the price of an empty file rather
    // than after the download. Doing, not predicting: metadata lies about
    // ACLs and read-only mounts; a create does not. (`--dry-run` returned
    // above — it writes nothing at all, probe included.)
    if !dpkg {
        let probe = dir.join(".self-update-probe");
        std::fs::write(&probe, b"")
            .map(|_| {
                let _ = std::fs::remove_file(&probe);
            })
            .map_err(|e| {
                DepotError::new(if e.kind() == std::io::ErrorKind::PermissionDenied {
                    format!(
                        "{} is not writable by you — re-run with the privileges \
                         that own it, or move the install somewhere you own",
                        dir.display()
                    )
                } else {
                    format!("{}: {e}", dir.display())
                })
            })?;
    }

    let fetched = net.artifact(&row.url, MAX_ARTIFACT_BYTES);
    let Some(body) = fetched.value else {
        return Err(DepotError::new(if fetched.reachable {
            format!("could not download {}: {}", row.name, fetched.why)
        } else {
            format!("could not reach {}: {}", row.url, fetched.why)
        }));
    };
    let got_hash = sha_hex(&body);
    if got_hash != want_hash {
        // The refusal that is the point of the whole verb. Nothing was
        // staged; the install is exactly as it was.
        return Err(DepotError::new(format!(
            "{} does not match what the origin published.\n  published  {want_hash}\n  \
             downloaded {got_hash}\nnothing was staged and nothing changed. If this \
             repeats, someone between you and the origin is editing downloads.",
            row.name
        )));
    }
    if !json {
        println!("downloaded and verified {} ({})", row.name, human(body.len() as i64));
    }

    if let Container::Deb = container {
        // dpkg's half. Ours ends at a verified file and the exact command.
        let cache = state_root.join("self-update");
        std::fs::create_dir_all(&cache)
            .map_err(|e| DepotError::new(format!("{}: {e}", cache.display())))?;
        let file = cache.join(basename(&row.name));
        std::fs::write(&file, &body)
            .map_err(|e| DepotError::new(format!("{}: {e}", file.display())))?;
        if !json {
            println!(
                "this install is the package manager's ({}), so the swap is its job:",
                dir.display()
            );
            println!("  sudo apt install {}", file.display());
            println!("the file above is already hash-checked against what the origin published.");
        }
        return say(
            json,
            12,
            "handoff",
            serde_json::json!({ "deb": file.to_string_lossy(), "command":
                format!("sudo apt install {}", file.display()) }),
        );
    }

    let files = match container {
        Container::TarGz => {
            let tar = gunzip(&body, MAX_MEMBER_BYTES).map_err(DepotError::new)?;
            tar_members(&tar).map_err(DepotError::new)?
        }
        Container::Zip => zip_members(&body).map_err(DepotError::new)?,
        Container::Deb => unreachable!("handled above"),
    };
    let cli_name = exe("scry");
    let gui_name = exe("scry-gui");
    let Some(cli_bytes) = member(&files, &cli_name) else {
        return Err(DepotError::new(format!(
            "{} holds no {cli_name} — not our artifact, nothing changed",
            row.name
        )));
    };
    // The window is replaced when it is installed here; absent stays absent —
    // an updater that ADDS programs has changed jobs.
    let gui_installed = dir.join(&gui_name).is_file();
    let gui_bytes = member(&files, &gui_name);
    if gui_installed && gui_bytes.is_none() {
        return Err(DepotError::new(format!(
            "{} holds no {gui_name}, and this install has one — refusing a \
             half-updated pair",
            row.name
        )));
    }

    // Stage everything, prove the CLI runs, then land the renames together —
    // the half-updated window is two renames wide, not a download wide. A
    // refusal in this phase takes its `.new` files with it, so "nothing
    // changed" describes the directory and not just the binaries.
    let prepared = (|| {
        let staged_cli = stage(&dir, &cli_name, cli_bytes)?;
        if gui_installed {
            stage(&dir, &gui_name, gui_bytes.expect("checked above"))?;
        }
        smoke(&staged_cli)
    })();
    if let Err(e) = prepared {
        for name in [&cli_name, &gui_name] {
            let _ = std::fs::remove_file(dir.join(format!("{name}.new")));
        }
        return Err(DepotError::new(e));
    }
    commit(&dir, &cli_name).map_err(DepotError::new)?;
    let mut replaced = vec![dir.join(&cli_name)];
    if gui_installed {
        commit(&dir, &gui_name).map_err(DepotError::new)?;
        replaced.push(dir.join(&gui_name));
    }

    if !json {
        for p in &replaced {
            println!("replaced {}", p.display());
        }
        println!(
            "{mine} is kept beside it as .old — renaming it back is the whole rollback.\n\
             this process is still {mine}; the next start is {there}."
        );
    }
    say(
        json,
        0,
        "updated",
        serde_json::json!({
            "from": mine, "to": there,
            "replaced": replaced.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        }),
    )
}

// ── proofs ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // A gzip envelope around a real DEFLATE stream, trailer zeroed — the
    // parser must not care, the sha256 upstream owns integrity.
    fn gz(payload: &[u8], flags: u8, decor: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 255];
        out.extend_from_slice(decor);
        out.extend(miniz_oxide::deflate::compress_to_vec(payload, 6));
        out.extend_from_slice(&[0u8; 8]);
        out
    }

    #[test]
    fn gunzip_roundtrips_and_honours_the_optional_fields() {
        let body = b"the bytes the bytes the bytes".repeat(50);
        assert_eq!(gunzip(&gz(&body, 0, b""), 1 << 20).unwrap(), body);
        // FNAME + FCOMMENT, both NUL-terminated, as a foreign packer may write.
        assert_eq!(
            gunzip(&gz(&body, 0x08 | 0x10, b"a-name\0a comment\0"), 1 << 20).unwrap(),
            body
        );
        // FEXTRA with a 4-byte field.
        assert_eq!(
            gunzip(&gz(&body, 0x04, &[4, 0, 1, 2, 3, 4]), 1 << 20).unwrap(),
            body
        );
    }

    #[test]
    fn gunzip_refuses_what_it_should() {
        assert!(gunzip(b"not gzip at all, far too short?", 1 << 20)
            .unwrap_err()
            .contains("magic"));
        let body = vec![7u8; 4096];
        // The limit is a wall, not advice.
        assert!(gunzip(&gz(&body, 0, b""), 100).unwrap_err().contains("inflate"));
    }

    // One tar member, headers written the way our packager writes them.
    fn tar_entry(name: &str, data: &[u8], typeflag: u8) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(b"0000755");
        let size = format!("{:011o}", data.len());
        h[124..135].copy_from_slice(size.as_bytes());
        h[156] = typeflag;
        h[257..262].copy_from_slice(b"ustar");
        // A believable checksum, though the parser does not read it.
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|b| *b as u32).sum();
        h[148..154].copy_from_slice(format!("{sum:06o}").as_bytes());
        h[154] = 0;
        h[155] = b' ';
        let mut out = h;
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
        out
    }

    fn tar_of(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut t = Vec::new();
        for (n, d, f) in entries {
            t.extend(tar_entry(n, d, *f));
        }
        t.extend_from_slice(&[0u8; 1024]);
        t
    }

    #[test]
    fn tar_walks_files_and_skips_directories() {
        let t = tar_of(&[
            ("./scry-9.9.9-linux-x86_64/", b"", b'5'),
            ("./scry-9.9.9-linux-x86_64/scry", b"#!/bin/sh\nexit 0\n", b'0'),
            ("./scry-9.9.9-linux-x86_64/scry-gui", b"#!/bin/sh\nexit 1\n", b'0'),
        ]);
        let files = tar_members(&t).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(member(&files, "scry").unwrap(), b"#!/bin/sh\nexit 0\n");
        assert_eq!(member(&files, "scry-gui").unwrap(), b"#!/bin/sh\nexit 1\n");
        // And the versioned root directory never mattered.
        assert!(member(&files, "absent").is_none());
    }

    #[test]
    fn tar_honours_the_gnu_longname_record() {
        let long = format!("./{}/scry", "d".repeat(120));
        let t = tar_of(&[
            ("././@LongLink", long.as_bytes(), b'L'),
            ("./truncated-name", b"payload", b'0'),
        ]);
        let files = tar_members(&t).unwrap();
        assert_eq!(files[0].0, long);
        assert_eq!(files[0].1, b"payload");
    }

    #[test]
    fn tar_refuses_a_member_past_the_end() {
        let mut t = tar_entry("x", b"1234", b'0');
        t.truncate(513); // header promises 4 bytes, archive holds one
        assert!(tar_members(&t).unwrap_err().contains("past the end"));
    }

    // A zip with one stored and one DEFLATE member, central directory and
    // EOCD included — the containers our packager writes, minus determinism.
    fn zip_of(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut count = 0u16;
        for (name, data, deflate) in entries {
            let local_at = out.len() as u32;
            let (method, packed): (u16, Vec<u8>) = if *deflate {
                (8, miniz_oxide::deflate::compress_to_vec(data, 6))
            } else {
                (0, data.to_vec())
            };
            out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]); // time, date, crc (unread)
            out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&packed);

            central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]); // extra, comment, disk, attrs
            central.extend_from_slice(&local_at.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
            count += 1;
        }
        let cd_at = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_at.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn zip_walks_stored_and_deflate_members() {
        let z = zip_of(&[
            ("scry.exe", b"MZ fake windows cli", false),
            ("scry-gui.exe", b"MZ fake windows gui, but longer and repeated MZ MZ MZ", true),
        ]);
        let files = zip_members(&z).unwrap();
        assert_eq!(member(&files, "scry.exe").unwrap(), b"MZ fake windows cli");
        assert!(member(&files, "scry-gui.exe").unwrap().starts_with(b"MZ fake"));
    }

    #[test]
    fn zip_refuses_truncation_and_unknown_methods() {
        let z = zip_of(&[("scry.exe", b"payload", true)]);
        assert!(zip_members(&z[..z.len() - 4]).is_err());
        let mut bad = zip_of(&[("scry.exe", b"payload", false)]);
        // Flip the method in both headers to 99.
        bad[8] = 99;
        let cd = bad.len() - 22 - 46 - "scry.exe".len();
        bad[cd + 10] = 99;
        assert!(zip_members(&bad).unwrap_err().contains("method 99"));
    }

    #[test]
    fn the_card_reads_the_documented_shape_and_reasonable_variants() {
        let host = "https://scry.example";
        // Rows as an array, path-relative urls, sha256 under two spellings.
        let card = parse_card(
            &json!({
                "client": { "version": "0.5.0" },
                "get_it": [
                    { "name": "scry-0.5.0-linux-x86_64.tar.gz",
                      "path": "/dl/scry-0.5.0-linux-x86_64.tar.gz",
                      "sha256": "aa".repeat(32), "bytes": 7 },
                    { "file": "scry_0.5.0_amd64.deb",
                      "url": "https://cdn.scry.example/dl/scry_0.5.0_amd64.deb",
                      "sha_256": "bb".repeat(32) },
                    // No name anywhere: the url's last segment serves.
                    { "href": "dl/scry-0.5.0-windows-x86_64.zip",
                      "sha256": "cc".repeat(32) },
                    // Protocol-relative: dropped, same rule as Manifest::rebase.
                    { "name": "evil", "url": "//elsewhere.example/x.tar.gz",
                      "sha256": "dd".repeat(32) },
                ],
            }),
            host,
        );
        assert_eq!(card.version.as_deref(), Some("0.5.0"));
        assert_eq!(card.rows.len(), 3);
        assert_eq!(
            card.rows[0].url,
            "https://scry.example/dl/scry-0.5.0-linux-x86_64.tar.gz"
        );
        assert_eq!(card.rows[0].sha256.as_deref(), Some("aa".repeat(32).as_str()));
        assert_eq!(card.rows[0].bytes, Some(7));
        assert!(card.rows[1].url.starts_with("https://cdn."));
        assert_eq!(card.rows[1].sha256.as_deref(), Some("bb".repeat(32).as_str()));
        assert_eq!(card.rows[2].name, "scry-0.5.0-windows-x86_64.zip");
    }

    #[test]
    fn the_card_survives_rows_keyed_as_an_object() {
        let card = parse_card(
            &json!({
                "client": { "version": "0.5.0" },
                "get_it": {
                    "deb": { "url": "/dl/scry_0.5.0_amd64.deb", "sha256": "aa".repeat(32) },
                },
            }),
            "https://scry.example",
        );
        assert_eq!(card.rows.len(), 1);
        assert_eq!(card.rows[0].name, "scry_0.5.0_amd64.deb");
    }

    #[test]
    fn a_row_with_a_sha_key_that_is_not_a_hash_carries_none() {
        let card = parse_card(
            &json!({ "get_it": [
                { "url": "/dl/scry-0.5.0-linux-x86_64.tar.gz", "sha256": "not hex" }
            ]}),
            "https://scry.example",
        );
        // Parsed liberally, refused later BY NAME at install time — never
        // silently installed unchecked.
        assert_eq!(card.rows[0].sha256, None);
    }

    fn rows(names: &[&str]) -> Vec<Row> {
        names
            .iter()
            .map(|n| Row {
                name: n.to_string(),
                url: format!("https://x/{n}"),
                sha256: Some("aa".repeat(32)),
                bytes: None,
            })
            .collect()
    }

    #[test]
    fn each_platform_picks_its_own_artifact() {
        let all = rows(&[
            "scry_0.5.0_amd64.deb",
            "scry-0.5.0-linux-x86_64.tar.gz",
            "scry-0.5.0-windows-x86_64.zip",
            "scry-0.5.0-macos-universal.tar.gz",
            "SHA256SUMS",
        ]);
        let (r, c) = pick(&all, "linux-x86_64", false).unwrap();
        assert_eq!((r.name.as_str(), c), ("scry-0.5.0-linux-x86_64.tar.gz", Container::TarGz));
        let (r, c) = pick(&all, "linux-x86_64", true).unwrap();
        assert_eq!((r.name.as_str(), c), ("scry_0.5.0_amd64.deb", Container::Deb));
        let (r, c) = pick(&all, "win-x86_64", false).unwrap();
        assert_eq!((r.name.as_str(), c), ("scry-0.5.0-windows-x86_64.zip", Container::Zip));
        for mac in ["mac-arm64", "mac-x86_64"] {
            let (r, c) = pick(&all, mac, false).unwrap();
            assert_eq!(
                (r.name.as_str(), c),
                ("scry-0.5.0-macos-universal.tar.gz", Container::TarGz)
            );
        }
    }

    #[test]
    fn a_missing_artifact_names_the_platform_and_what_the_card_has() {
        let e = pick(&rows(&["scry_0.5.0_amd64.deb"]), "linux-aarch64", false).unwrap_err();
        assert!(e.contains("linux-aarch64") && e.contains("scry_0.5.0_amd64.deb"), "{e}");
        let e = pick(&rows(&[]), "linux-x86_64", false).unwrap_err();
        assert!(e.contains("no downloads at all"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn stage_and_commit_swap_the_binary_and_keep_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("scry");
        std::fs::write(&target, b"old bytes").unwrap();

        stage(dir.path(), "scry", b"#!/bin/sh\nexit 0\n").unwrap();
        smoke(&dir.path().join("scry.new")).unwrap();
        commit(dir.path(), "scry").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"#!/bin/sh\nexit 0\n");
        assert_eq!(
            std::fs::read(dir.path().join("scry.old")).unwrap(),
            b"old bytes",
            "the rollback copy is the previous binary"
        );
        assert!(!dir.path().join("scry.new").exists(), "nothing staged is left");
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_binary_that_fails_its_smoke_run_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let staged = stage(dir.path(), "scry", b"#!/bin/sh\nexit 9\n").unwrap();
        let e = smoke(&staged).unwrap_err();
        assert!(e.contains("smoke"), "{e}");
    }
}
