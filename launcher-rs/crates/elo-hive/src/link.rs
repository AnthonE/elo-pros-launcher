//! Linking this machine's chat key to a wallet, and unlinking it — the client
//! half of `meter/hive.py` `/hive/link`, written once for the Chat window and
//! `elo chat link`.
//!
//! The link is two signatures: the wallet signs a play text naming the key,
//! and the key signs a NIP-01 event naming the wallet. Before either, the
//! wallet signs the text its chat key is DERIVED from (`derivation_message`),
//! so the key linked is the one elopros.com makes from the same wallet — one
//! wallet, one identity, whichever client it speaks from. Unlinking is one
//! signature: the wallet's, over the same play text naming `-`.
//!
//! **Every text the wallet signs is rebuilt here** and must equal the one the
//! origin offers, so nothing is signed on the origin's word. How a text is
//! shown and approved is the caller's: a window shows a dialog, a terminal
//! prints it and asks, a paired browser shows it on its own screen. This
//! module never signs — it asks [`Sign`] — and it never deletes a key.

use crate::voice::{derivation_message, link_content, UnsignedEvent, Voice, KEY_NAME};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The play action the wallet signs (`meter/playauth.py` ACTIONS).
pub const ACTION: &str = "chatkey";

/// The origin's half, behind a seam so the flow runs with no network.
pub trait LinkHttp {
    /// `GET /api/hive/link?wallet=…[&npub=…]` — the texts, and what is linked.
    fn quote(&self, host: &str, wallet: &str, npub: Option<&str>) -> elo_net::Fetched<Value>;
    /// `POST /api/hive/link` — the signatures.
    fn link(&self, host: &str, body: &Value) -> elo_net::Fetched<Value>;
}

pub struct RealLinkHttp;

impl LinkHttp for RealLinkHttp {
    fn quote(&self, host: &str, wallet: &str, npub: Option<&str>) -> elo_net::Fetched<Value> {
        let mut url = format!("{host}/api/hive/link?wallet={}", crate::who::urlencode(wallet));
        if let Some(n) = npub {
            url.push_str("&npub=");
            url.push_str(&crate::who::urlencode(n));
        }
        elo_net::Net::new().get_json_quick(&url)
    }
    fn link(&self, host: &str, body: &Value) -> elo_net::Fetched<Value> {
        elo_net::Net::new().post_json_told(&format!("{host}/api/hive/link"), body)
    }
}

/// One signature the wallet is asked for.
pub struct Request<'a> {
    /// Exactly what is signed.
    pub text: &'a str,
    /// A few words, for a signer that shows a reason (a paired browser).
    pub why: &'static str,
    /// What signing it does, in a sentence, for whatever shows the text.
    pub about: &'static str,
    /// The word on the button that signs it.
    pub button: &'static str,
}

/// The wallet signs `request.text` and answers its EIP-191 signature (0x hex),
/// or why not. Showing the text first is the caller's job.
pub type Sign<'a> = dyn FnMut(&Request) -> Result<String, String> + 'a;

const DERIVE: (&str, &str, &str) = (
    "derive this wallet's chat key",
    "It makes this wallet's chat key, the same one elopros.com makes. A second text, \
     naming that key, is shown before it is signed.",
    "Sign",
);
const LINK: (&str, &str, &str) = (
    "link this chat key to the wallet",
    "It links the key to your wallet. The link is public: people in chat see your \
     profile name and picture beside what you say.",
    "Sign and link",
);
const UNLINK: (&str, &str, &str) = (
    "unlink this wallet's chat key",
    "It unlinks your wallet's chat key. Rooms stop showing your profile name and \
     picture; the key stays on this machine, and what it said stays where it was said.",
    "Sign and unlink",
);

fn ask(sign: &mut Sign, text: &str, (why, about, button): (&'static str, &'static str, &'static str))
    -> Result<String, String> {
    sign(&Request { text, why, about, button })
}

/// The signed detail — byte-for-byte `meter/hive.py` `link_detail`: sorted,
/// compact JSON of the `at` and the npub (or `-`).
pub fn detail(at: i64, npub: &str) -> String {
    json!({"at": at, "npub": npub}).to_string()
}

/// The play text the wallet signs to link `npub` (or `-` to unlink) —
/// `meter/playauth.py` `play_message("chatkey", wallet, detail, day_of(at))`.
pub fn sign_text(wallet: &str, at: i64, npub: &str) -> String {
    format!(
        "elo play\naction: {ACTION}\nwallet: {}\nday: {}\ndetail: {}",
        wallet.trim().to_lowercase(),
        crate::clock::day(at),
        detail(at, npub)
    )
}

/// Where the old key goes when the wallet's key replaces it: kept beside the
/// new one, never deleted, because it is the only copy of whatever was said
/// under it.
pub fn retired_key_name(old_npub: &str) -> String {
    let tag: String = old_npub.trim_start_matches("npub1").chars().take(10).collect();
    format!("{KEY_NAME}.{tag}.old")
}

fn wallet_of(wallet: &str) -> Result<String, String> {
    let w = wallet.trim().to_lowercase();
    let hex = w.strip_prefix("0x").unwrap_or("");
    if hex.len() == 40 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(w)
    } else {
        Err(format!("{} is not a wallet — 0x and forty hex digits", crate::render::safe_line(wallet)))
    }
}

fn quote(http: &dyn LinkHttp, host: &str, wallet: &str, npub: Option<&str>)
    -> Result<Value, String> {
    let got = http.quote(host, wallet, npub);
    got.value.ok_or_else(|| {
        if got.reachable {
            format!("the origin would not answer — {}", got.why)
        } else {
            format!("could not reach {host} — {}", got.why)
        }
    })
}

/// The `at` the origin chose and the play text, checked against our rebuild.
fn offered(q: &Value, wallet: &str, npub: &str) -> Result<(i64, String), String> {
    let at = q.get("at").and_then(Value::as_i64).ok_or("the origin returned no `at` to sign")?;
    let text = sign_text(wallet, at, npub);
    if q.get("sign_this").and_then(Value::as_str) != Some(text.as_str()) {
        return Err("The origin offered a different text from the one this launcher builds, \
                    so nothing was signed. Update the launcher and try again.".into());
    }
    Ok((at, text))
}

fn post(http: &dyn LinkHttp, host: &str, body: &Value) -> Result<Value, String> {
    let sent = http.link(host, body);
    let Some(v) = sent.value else {
        return Err(if sent.reachable {
            format!("the origin would not answer — {}", sent.why)
        } else {
            format!("could not reach {host} — {}", sent.why)
        });
    };
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        return Err(format!("the origin refused — {}", crate::render::safe_line(e)));
    }
    if !sent.ok {
        return Err(format!("the origin refused — {}", sent.why));
    }
    Ok(v)
}

/// What a wallet has linked now, as an npub — `Ok(None)` is "nothing", and
/// a failed look is an `Err`, never a `None`.
pub fn linked(http: &dyn LinkHttp, host: &str, wallet: &str) -> Result<Option<String>, String> {
    let wallet = wallet_of(wallet)?;
    let q = quote(http, host, &wallet, None)?;
    Ok(q.get("linked").and_then(Value::as_str).map(str::to_string))
}

/// What [`link`] did.
#[derive(Debug)]
pub struct Linked {
    pub npub: String,
    /// The key that was on this machine before, kept here.
    pub kept: Option<PathBuf>,
    /// The key the wallet had linked before, if another.
    pub was: Option<String>,
}

/// Link the wallet's own chat key, and make it this machine's.
///
/// Three signatures from the wallet's side, every one shown by `sign` first:
/// the derivation text (whose signature IS the key), then the play text naming
/// that key. The key signs the third, the event naming the wallet. Everything
/// that can be refused first is — an origin that will not answer, a text that
/// differs from the one the website uses — and the key is saved only after
/// the origin has recorded the link.
pub fn link(http: &dyn LinkHttp, host: &str, wallet: &str, voice_dir: &Path, sign: &mut Sign)
    -> Result<Linked, String> {
    let wallet = wallet_of(wallet)?;
    let q0 = quote(http, host, &wallet, None)?;
    let derive_text = derivation_message(&wallet);
    if q0.pointer("/voice_key/sign_this").and_then(Value::as_str) != Some(derive_text.as_str()) {
        return Err("The origin handed back a different key text than the website uses, \
                    so the key would not be your wallet's. Nothing was signed — update the \
                    launcher and try again.".into());
    }
    let sig = ask(sign, &derive_text, DERIVE)?;
    let voice = Voice::from_wallet_signature(&sig).map_err(|e| e.to_string())?;
    let npub = voice.npub();

    let q1 = quote(http, host, &wallet, Some(&npub))?;
    if q1.get("event_content").and_then(Value::as_str) != Some(link_content(&wallet).as_str()) {
        return Err("the origin's link statement is not the one this launcher knows — \
                    nothing was linked".into());
    }
    let (at, text) = offered(&q1, &wallet, &npub)?;
    // The derivation signature never left this machine, so a stop here leaves
    // nothing anywhere: no link, no key saved.
    let wsig = ask(sign, &text, LINK).map_err(|e| {
        if e.starts_with("Cancelled") {
            "Cancelled — nothing was linked, and the key was not saved.".to_string()
        } else {
            format!("{e} — nothing was linked, and the key was not saved.")
        }
    })?;
    let event = voice
        .sign(UnsignedEvent {
            created_at: crate::clock::now(),
            kind: 1,
            tags: Vec::new(),
            content: link_content(&wallet),
        })
        .map_err(|e| e.to_string())?;
    let v = post(http, host, &json!({
        "wallet": wallet, "npub": npub, "at": at, "signature": wsig, "event": event.to_json(),
    }))?;
    if v.get("npub").and_then(Value::as_str) != Some(npub.as_str()) {
        return Err("the origin's reply names a different key — nothing was saved".into());
    }

    // The wallet's key becomes this machine's chat key. One that was here
    // before is kept, renamed, beside it.
    let mut kept = None;
    if let Ok(old) = Voice::load(voice_dir) {
        if old.npub() != npub {
            let to = voice_dir.join(retired_key_name(&old.npub()));
            std::fs::rename(voice_dir.join(KEY_NAME), &to)
                .map_err(|e| format!("linked, but the old key could not be moved aside: {e}"))?;
            kept = Some(to);
        }
    }
    voice.save(voice_dir).map_err(|e| format!("linked, but the key could not be saved: {e}"))?;
    let was = v.get("was").and_then(Value::as_str).filter(|w| *w != npub).map(str::to_string);
    Ok(Linked { npub, kept, was })
}

/// Unlink whatever chat key the wallet has linked. One signature, shown by
/// `sign` first. The key stays on this machine; only the public record goes.
/// Answers the npub that was linked.
pub fn unlink(http: &dyn LinkHttp, host: &str, wallet: &str, sign: &mut Sign)
    -> Result<String, String> {
    let wallet = wallet_of(wallet)?;
    let q = quote(http, host, &wallet, Some("-"))?;
    let Some(was) = q.get("linked").and_then(Value::as_str).map(str::to_string) else {
        return Err("No chat key is linked to this wallet, so there is nothing to unlink.".into());
    };
    let (at, text) = offered(&q, &wallet, "-")?;
    let sig = ask(sign, &text, UNLINK)?;
    let v = post(http, host, &json!({"wallet": wallet, "npub": "-", "at": at, "signature": sig}))?;
    if !v.get("npub").is_some_and(Value::is_null) {
        return Err("the origin's reply still names a key — check `elo chat whoami`".into());
    }
    Ok(was)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const WALLET: &str = "0x3e20ab25eb721dd4b691516238df14f0f5d3f7a3";
    /// `eth_account`'s signature of the derivation text with key 0x7a×32 —
    /// the vector `tests/voice.rs` and the elo-ui vault test share.
    const DERIVED: &str = "0x85194638f30fce1ede1041f70e85a9cc94841f813babc1e92468dd5ee25cece8\
                           48675754662b9a2c319cfc6f2af920106a74547c8920dd5928cc5078578a25d51b";
    const NPUB: &str = "npub1k7dy53fkr25pupyjxqn64aytu8v792v96l9p2f7gl3edk424ljrsnqjp92";

    /// An origin that answers like `/hive/link`, with what it was sent kept.
    struct Desk {
        reachable: bool,
        linked: Option<&'static str>,
        /// Make the play text it offers differ from the one a client builds.
        lie: bool,
        posted: RefCell<Vec<Value>>,
    }

    impl Desk {
        fn new() -> Desk {
            Desk { reachable: true, linked: None, lie: false, posted: RefCell::new(Vec::new()) }
        }
    }

    impl LinkHttp for Desk {
        fn quote(&self, _: &str, wallet: &str, npub: Option<&str>) -> elo_net::Fetched<Value> {
            if !self.reachable {
                return elo_net::Fetched::unreachable("dns error");
            }
            let mut q = json!({"wallet": wallet, "linked": self.linked,
                               "voice_key": {"sign_this": derivation_message(wallet)}});
            if let Some(n) = npub {
                let at = 1_790_000_000;
                let text = if self.lie { sign_text(wallet, at, "npub1someoneelse") }
                           else { sign_text(wallet, at, n) };
                q["npub"] = json!(n);
                q["at"] = json!(at);
                q["sign_this"] = json!(text);
                if n != "-" {
                    q["event_content"] = json!(link_content(wallet));
                }
            }
            elo_net::Fetched::good(q)
        }
        fn link(&self, _: &str, body: &Value) -> elo_net::Fetched<Value> {
            self.posted.borrow_mut().push(body.clone());
            let n = body["npub"].as_str().unwrap_or("");
            elo_net::Fetched::good(if n == "-" {
                json!({"wallet": body["wallet"], "npub": null, "was": self.linked})
            } else {
                json!({"wallet": body["wallet"], "npub": n, "was": self.linked})
            })
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("elo-link-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_play_text_is_the_meters_byte_for_byte() {
        // meter: play_message("chatkey", w, link_detail(1790000000, "-"), day_of(1790000000))
        assert_eq!(
            sign_text("0x3E20AB25EB721DD4B691516238DF14F0F5D3F7A3", 1_790_000_000, "-"),
            "elo play\naction: chatkey\nwallet: 0x3e20ab25eb721dd4b691516238df14f0f5d3f7a3\n\
             day: 2026-09-21\ndetail: {\"at\":1790000000,\"npub\":\"-\"}"
        );
    }

    #[test]
    fn a_link_signs_the_derivation_then_the_link_and_saves_the_wallets_key() {
        let d = dir("ok");
        let old = Voice::generate().unwrap();
        old.save(&d).unwrap();
        let desk = Desk::new();
        let shown = RefCell::new(Vec::new());
        let mut sign = |r: &Request| {
            shown.borrow_mut().push(r.text.to_string());
            Ok(if r.text.starts_with("scry hive voice") { DERIVED.into() } else { "0xsig".into() })
        };
        let done = link(&desk, "https://x", WALLET, &d, &mut sign).unwrap();
        assert_eq!(done.npub, NPUB);
        let shown = shown.borrow();
        assert_eq!(shown[0], derivation_message(WALLET));
        assert_eq!(shown[1], sign_text(WALLET, 1_790_000_000, NPUB));
        let body = &desk.posted.borrow()[0];
        assert_eq!(body["signature"], "0xsig");
        assert_eq!(body["event"]["content"], link_content(WALLET));
        assert_eq!(Voice::load(&d).unwrap().npub(), NPUB, "the wallet's key is this machine's now");
        assert_eq!(done.kept.as_deref(), Some(d.join(retired_key_name(&old.npub())).as_path()));
        assert!(done.kept.unwrap().is_file(), "the old key is kept, not deleted");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_text_the_origin_changed_is_never_signed() {
        let d = dir("lie");
        let desk = Desk { lie: true, ..Desk::new() };
        let mut asked = 0;
        let mut sign = |r: &Request| {
            asked += 1;
            assert!(r.text.starts_with("scry hive voice"), "signed a text the origin changed");
            Ok(DERIVED.to_string())
        };
        let e = link(&desk, "https://x", WALLET, &d, &mut sign).unwrap_err();
        assert!(e.contains("different text"), "{e}");
        assert_eq!(asked, 1);
        assert!(desk.posted.borrow().is_empty());
        assert!(!d.join(KEY_NAME).exists(), "no key saved for a link that never happened");
    }

    #[test]
    fn a_cancel_at_the_link_text_saves_nothing() {
        let d = dir("cancel");
        let desk = Desk::new();
        let mut sign = |r: &Request| {
            if r.text.starts_with("scry hive voice") { Ok(DERIVED.into()) }
            else { Err("Cancelled — nothing was signed.".into()) }
        };
        let e = link(&desk, "https://x", WALLET, &d, &mut sign).unwrap_err();
        assert_eq!(e, "Cancelled — nothing was linked, and the key was not saved.");
        assert!(desk.posted.borrow().is_empty());
        assert!(!d.join(KEY_NAME).exists());
    }

    #[test]
    fn unlink_signs_one_text_naming_dash_and_sends_no_event() {
        let desk = Desk { linked: Some(NPUB), ..Desk::new() };
        let mut shown = Vec::new();
        let mut sign = |r: &Request| {
            shown.push(r.text.to_string());
            Ok("0xsig".to_string())
        };
        assert_eq!(unlink(&desk, "https://x", WALLET, &mut sign).unwrap(), NPUB);
        assert_eq!(shown, vec![sign_text(WALLET, 1_790_000_000, "-")]);
        let body = &desk.posted.borrow()[0];
        assert_eq!(body["npub"], "-");
        assert!(body.get("event").is_none());
    }

    #[test]
    fn unlink_with_nothing_linked_or_no_origin_asks_for_nothing() {
        let mut sign = |_: &Request| -> Result<String, String> { panic!("asked to sign") };
        let e = unlink(&Desk::new(), "https://x", WALLET, &mut sign).unwrap_err();
        assert!(e.contains("nothing to unlink"), "{e}");
        let gone = Desk { reachable: false, ..Desk::new() };
        let e = unlink(&gone, "https://x", WALLET, &mut sign).unwrap_err();
        assert!(e.contains("could not reach"), "{e}");
        let e = unlink(&gone, "https://x", "0xnope", &mut sign).unwrap_err();
        assert!(e.contains("not a wallet"), "{e}");
    }
}
