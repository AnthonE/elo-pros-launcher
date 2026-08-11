//! The scry overlay protocol, v1 — `sdk/PROTOCOL.md` is the spec.
//!
//! Newline-delimited JSON, one request object per line, one reply per line, in
//! order, on one connection. No framing header, no length prefix, no handshake
//! beyond `hello`.
//!
//! # The two rules that are not conveniences
//!
//! **Fields are allowlisted per verb, and an unknown field is a REFUSAL rather
//! than an ignore.** A caller that believes it set something and was quietly
//! ignored has been misled in the dangerous direction — it is the same rule
//! `arca.py` states, for the same reason. A game that sends
//! `{"op":"sign","text":"…","budget":999}` must be told `budget` means nothing
//! here, not have it dropped.
//!
//! **Every verb refuses until `hello` has named the game**, because a consent
//! prompt with no name in it is not consent.

use serde_json::{json, Map, Value};

/// The protocol version this implementation speaks. A `hello` naming another
/// one is refused rather than negotiated: there is one version, and a client
/// that guessed wrong should be told so.
pub const PROTOCOL: i64 = 1;

/// The whole vocabulary, with the fields each verb accepts.
///
/// Note what is absent, and it is the design rather than an omission: nothing
/// installs, uninstalls, launches another title, reads a file, writes a
/// setting, chooses a signer, or names a signing command. **A game may ask
/// about itself and ask for a signature; it may not operate the launcher.**
/// ⚠ **This is the only broker, and it was not always.** A second one existed
/// in Python until 2026-08-07, and `prove` shipped here, in both SDK clients and
/// in `sdk/PROTOCOL.md` while that one refused it as an unknown op — dark to
/// half the players with every suite green, because each half was only ever
/// driven against its own kind. Deleting it removed the bug class.
///
/// What is still two-sided is the **spec** and the **clients**:
/// `sdk/test_sdk.py` asserts that every verb advertised here is documented in
/// `sdk/PROTOCOL.md` and callable from both SDK clients. Adding a verb without
/// those is adding one nobody can discover or use.
pub const VERBS: [(&str, &[&str]); 9] = [
    ("hello", &["op", "game", "protocol", "version"]),
    ("identity", &["op"]),
    ("sign", &["op", "text", "why"]),
    // `prove` takes NO text, and that absence is the whole verb — see
    // `prove_message`. The game hands over two opaque values and the launcher
    // writes the sentence.
    ("prove", &["op", "nonce", "server"]),
    ("profile", &["op", "address"]),
    ("title", &["op", "slug"]),
    ("servers", &["op", "slug"]),
    ("open", &["op", "url"]),
    ("overlay", &["op", "slug"]),
];

/// `0x` + 40 hex digits, and nothing else gets interpolated into a url.
pub fn looks_like_address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// `/api/who` → the dozen fields a HUD draws.
///
/// A join, not a judgement — this reshapes and never decides. Two things it is
/// careful about:
///
/// **Sworn and self-declared are kept apart.** `handle` is the NIP-05 name out
/// of the locked index; `display_name` is whatever its owner typed into a face.
/// Merging them into one "name" would let anyone wear a checked name for free,
/// so they stay separate and `sworn` says which is which.
///
/// **One wallet can hold several vows.** `/who` answers with the first and
/// reports the count; `identities` carries it through so a game can say "one of
/// 3" rather than implying the wallet has exactly one identity.
///
/// Pinned to `sdk/testdata/who_fixture.json` by `tests/cross_language.rs`, and
/// the same fixture is checked against the live wire by `sdk/test_sdk.py`.
pub fn flatten_who(body: &Value, address: &str) -> Value {
    let found = body.get("found").cloned().unwrap_or(json!({}));
    let subject = body.get("subject").cloned().unwrap_or(json!({}));
    let reach = found.get("reach").cloned().unwrap_or(json!({}));
    let s = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);

    let nip05 = s(found.get("nip05"));
    let sandbox = found.get("sandbox").and_then(Value::as_bool).unwrap_or(false);
    let wallet = s(found.get("wallet"))
        .or_else(|| s(subject.get("wallet")))
        .unwrap_or_else(|| address.to_string());
    // A path on the origin becomes an /api path — the edge strips the prefix on
    // the way in, so a url written into a reply must carry it (`CLAUDE.md`
    // §traps: declare routes bare, write urls with `/api/`).
    let link = |p: Option<String>| -> Value {
        match p {
            Some(p) if !p.is_empty() => json!(format!("/api/{}", p.trim_start_matches('/'))),
            _ => Value::Null,
        }
    };
    let identities = subject
        .get("identities")
        .and_then(Value::as_i64)
        .or_else(|| body.get("matches").and_then(Value::as_i64))
        .unwrap_or(1);

    json!({
        "address": wallet,
        // the checked half
        "handle": nip05.clone(),
        "sworn": nip05.is_some() && !sandbox,
        "npub": s(found.get("npub")),
        "vow_id": s(found.get("vow_id")),
        // the self-declared half — a claim, and labelled as one
        "display_name": s(found.get("agent")),
        "face": found.get("face").cloned().unwrap_or(Value::Null),
        "avatar": link(s(found.get("avatar"))),
        // derived by the origin, and only ever certain for what the house hosts
        "kind": s(found.get("kind")),
        "sandbox": sandbox,
        "in_the_hive": reach.get("in_the_hive").and_then(Value::as_bool).unwrap_or(false),
        // a wallet is one party; the vows are what it swore
        "identities": identities,
        "authoritative": body.get("authoritative").and_then(Value::as_bool).unwrap_or(true),
        // the reads a game makes ITSELF, the way Steam splits
        // GetPlayerSummaries from GetPlayerAchievements. One verb, one read.
        "links": {
            "achievements": link(Some(format!("achievements/of/{wallet}"))),
            // The item shelf. It answers `configured: false` until a creator
            // deploys a ScryItem — an honest dark surface, not a broken read,
            // and a game should render the difference.
            "items": link(Some(format!("items/of/{wallet}"))),
            "holdings": link(Some(format!("holdings/{wallet}"))),
            "reputation": link(s(found.pointer("/reputation/at"))),
            "vow": link(s(found.pointer("/conduct/vow"))),
            "tip": link(s(reach.get("tip"))),
        },
    })
}

/// A refusal a player could be shown. `reason` is written to be read, never
/// parsed — the machine-readable part is `ok:false` plus the optional
/// discriminators below.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub reason: String,
    /// Set when the player themselves said no, so a game can stop asking.
    pub refused_by_player: bool,
    /// Set when the signer is the browser and the player finishes there. **Not
    /// an error** — a game that renders this as one tells the player something
    /// broke when nothing did.
    pub handoff: Option<String>,
}

impl Refusal {
    pub fn new(reason: impl Into<String>) -> Refusal {
        Refusal { reason: reason.into(), refused_by_player: false, handoff: None }
    }

    pub fn by_player(reason: impl Into<String>) -> Refusal {
        Refusal { reason: reason.into(), refused_by_player: true, handoff: None }
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("ok".into(), json!(false));
        m.insert("reason".into(), json!(self.reason));
        if self.refused_by_player {
            m.insert("refused_by".into(), json!("the player"));
        }
        if let Some(url) = &self.handoff {
            m.insert("handoff".into(), json!(url));
        }
        Value::Object(m)
    }
}

/// Check one request against the vocabulary before anything acts on it.
///
/// Returns the verb name, or the refusal to send back. This runs before the
/// session state is consulted so a malformed request is answered the same way
/// whether or not `hello` has happened — the shape of the door does not depend
/// on who is standing at it.
pub fn validate(req: &Value) -> Result<&'static str, Refusal> {
    let obj = req.as_object().ok_or_else(|| {
        Refusal::new("a request is a JSON object, one per line")
    })?;
    let op = obj
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal::new("every request needs an \"op\""))?;

    let (name, allowed) = VERBS
        .iter()
        .find(|(v, _)| *v == op)
        .ok_or_else(|| {
            let known: Vec<&str> = VERBS.iter().map(|(v, _)| *v).collect();
            Refusal::new(format!(
                "unknown op {op:?}. This door speaks: {}",
                known.join(", ")
            ))
        })?;

    // The allowlist. An unknown field is a refusal that NAMES the field, so a
    // caller can fix it rather than guess which one was wrong.
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(Refusal::new(format!(
                "{op}: unknown field {key:?}. This verb accepts: {}. \
                 An unknown field is refused rather than ignored, so a caller \
                 is never quietly misled about what it asked for.",
                allowed.join(", ")
            )));
        }
    }
    Ok(name)
}

/// The reply to a well-formed `hello`.
pub fn hello_reply(host: &str, signer: &str) -> Value {
    let caps: Vec<&str> = VERBS.iter().map(|(v, _)| *v).collect();
    json!({
        "ok": true,
        "protocol": PROTOCOL,
        "launcher": "scry-launcher",
        "host": host,
        "capabilities": caps,
        "signer": signer,
    })
}

/// `hello`'s own checks, kept apart from [`validate`] because they are about
/// the *values* rather than the shape.
pub fn check_hello(req: &Value) -> Result<String, Refusal> {
    let game = req
        .get("game")
        .and_then(Value::as_str)
        .filter(|g| !g.is_empty())
        .ok_or_else(|| {
            Refusal::new(
                "hello must name the game — a consent prompt with no name in \
                 it is not consent",
            )
        })?;
    if let Some(p) = req.get("protocol") {
        let p = p.as_i64().unwrap_or(-1);
        if p != PROTOCOL {
            return Err(Refusal::new(format!(
                "this launcher speaks protocol {PROTOCOL}, the game asked for {p}"
            )));
        }
    }
    check_slug(game)?;
    Ok(game.to_string())
}

/// A slug names a directory, a manifest key, and — since `prove` became SIWE —
/// a word inside a signed statement. The same shape the depot enforces,
/// refused here so a hostile name never reaches any of them.
///
/// Its own function because `scry prove` on the CLI needs exactly this check
/// and nothing else about a handshake; two copies of a rule that bounds what
/// goes into signed bytes is one copy too many.
pub fn check_slug(game: &str) -> Result<(), Refusal> {
    if !game.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || game.len() > 64
        || game.is_empty()
    {
        return Err(Refusal::new(format!(
            "{game:?} is not a slug — letters, digits, dash and underscore, \
             up to 64 characters"
        )));
    }
    Ok(())
}

/// `open` is https-only. A launcher that will open any url a game hands it is
/// a launcher that will open `file:///` and `javascript:`.
pub fn check_url(req: &Value) -> Result<String, Refusal> {
    let url = req
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal::new("open needs a url"))?;
    if !url.starts_with("https://") {
        return Err(Refusal::new(format!(
            "open is https only, got {url:?}"
        )));
    }
    Ok(url.to_string())
}

/// The family a `sign` request belongs to: the first line must read
/// `scry <family>`.
///
/// The family is what consent is counted against, so it is parsed from the
/// text the player will be shown rather than taken from a field beside it —
/// a field could disagree with the message and the message is what gets
/// signed.
/// RH-Chain. The one chain this town's identities live on
/// (`contracts/deployments.json` keys its whole record on it).
pub const CHAIN_ID: u64 = 4663;

/// The message `prove` signs: **Sign-In With Ethereum, EIP-4361**, composed
/// here and never supplied by the game.
///
/// # Why a standard rather than our own line format
///
/// Operator, 2026-08-07: *"i dont see why we have some special crypto things
/// that no one else has lol."* This used to emit
/// `scry prove\ngame:…\nserver:…\nnonce:…`, invented here. The cryptography was
/// never the bespoke part — EIP-191, secp256k1 and `ecrecover` are what
/// everyone uses — but the *text* was, and it landed on the one message a
/// **third party** has to verify. A game's backend had to hand-roll a string
/// concatenation and get it byte-exact, with a mismatch showing up as a silent
/// signature failure.
///
/// EIP-4361 is the standard for exactly this operation, and every backend
/// already has a library for it (`siwe` for JS/Rust/Python/Go, built into viem
/// and ethers). It is also strictly stronger than what it replaced: the domain
/// is bound into the signed bytes, and `Issued At` gives a verifier something
/// to age a proof against. The old format had neither.
///
/// # Why there is still no consent prompt
///
/// Unchanged, and it is the whole security argument: `sign` takes arbitrary
/// text from the game, so it needs a player in the loop — the game chooses the
/// bytes and a prompt is the only thing between a player and a sentence
/// somebody else wrote. `prove` inverts that. The game supplies a domain and a
/// nonce, two opaque values it cannot give meaning to, and the launcher writes
/// every word. There is nothing to consent to because the game cannot say
/// anything. That is `CLAUDE.md` invariant 8 at the right layer: we do not
/// sanitise the game's sentence, we refuse to let it write one.
///
/// `game` comes from the `hello` handshake and never from the request, so a
/// title cannot prove as somebody else's title.
///
/// ```text
/// shard-3.gates.example wants you to sign in with your Ethereum account:
/// 0x24445EFddB08d4938E3E3627042B2Cf4063d9092
///
/// Prove your identity to play gates. This signature authorises nothing and moves no funds.
///
/// URI: https://shard-3.gates.example
/// Version: 1
/// Chain ID: 4663
/// Nonce: 8f14e45fceea167a
/// Issued At: 2026-08-07T12:00:00Z
/// ```
///
/// A verifier recomputes this from what it already knows — the domain it
/// serves, the nonce it issued, the address it recovers — or, far more likely,
/// hands the whole string to a SIWE library. It must **not** trust the
/// `message` field off the wire; both sides deriving it independently is the
/// check.
///
/// `issued_at` is a parameter rather than read from the clock so the message is
/// a pure function and a test can pin it. Callers pass [`now_iso8601`].
pub fn prove_message(
    domain: &str,
    address: &str,
    game: &str,
    nonce: &str,
    issued_at: &str,
) -> String {
    // EIP-4361 §Message Format. The blank lines around `statement` are part of
    // the grammar, not formatting — a parser reads them.
    format!(
        "{domain} wants you to sign in with your Ethereum account:\n\
         {address}\n\
         \n\
         {}\n\
         \n\
         URI: https://{domain}\n\
         Version: 1\n\
         Chain ID: {CHAIN_ID}\n\
         Nonce: {nonce}\n\
         Issued At: {issued_at}",
        statement(game)
    )
}

/// The one human sentence in the message. **Must not contain a newline** —
/// EIP-4361 forbids it, and here that rule is also the injection guard, since
/// `game` comes off the `hello` handshake. `check_game` has already bounded it.
pub fn statement(game: &str) -> String {
    format!(
        "Prove your identity to play {game}. \
         This signature authorises nothing and moves no funds."
    )
}

/// The message `scry signin` composes to sign the WEBSITE in — the device-code
/// pairing (`meter/signin.py`, the origin's `/api/signin` router).
///
/// Same EIP-4361 grammar as [`prove_message`], different statement: this one
/// names a web browser rather than a game, so a player reading either knows
/// which door asked. The `nonce` is the pairing code off the browser's screen
/// — the player TYPED it, which is the anti-phish property: nothing clickable
/// can carry someone else's code into this composer.
///
/// The verifier is Python (`signin.verify_proof`), which recomposes these
/// exact bytes from its own domain, the code it issued and the address it
/// recovers. `sdk/testdata/signin_fixture.json` pins both sides; neither
/// moves without it.
pub fn signin_message(domain: &str, address: &str, nonce: &str, issued_at: &str) -> String {
    format!(
        "{domain} wants you to sign in with your Ethereum account:\n\
         {address}\n\
         \n\
         Sign in to {domain} in a web browser. \
         This signature authorises nothing and moves no funds.\n\
         \n\
         URI: https://{domain}\n\
         Version: 1\n\
         Chain ID: {CHAIN_ID}\n\
         Nonce: {nonce}\n\
         Issued At: {issued_at}"
    )
}

/// `2026-08-07T12:00:00Z` — EIP-4361 wants an ISO-8601 datetime.
///
/// Hand-rolled against `SystemTime` rather than pulling `chrono` in: this crate
/// carries `serde_json` and nothing else, and a date formatter is fifteen lines
/// of a public-domain algorithm (Howard Hinnant's civil-from-days, the same one
/// `scry-hive`'s clock uses).
pub fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso8601(secs)
}

pub fn iso8601(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let rem = unix_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The `domain` a game names — an RFC-3986 authority, `host` or `host:port`.
///
/// ⚠ **This is half the injection guard**, and the reason it is stricter than
/// the free-text field it replaced: the domain lands in the signed bytes twice
/// (the first line and the `URI:` line). A domain carrying a newline could
/// append a line to a message the launcher believes it authored — the exact
/// smuggling `prove` exists to prevent, reintroduced through the one gap a game
/// still controls. A scheme or a path would also make the `URI:` line a
/// different url from the one a verifier reconstructs.
pub fn check_domain(value: &str) -> Result<(), Refusal> {
    if value.is_empty() {
        return Err(Refusal::new("prove: server is empty — name the domain your \
                                 game server answers on, e.g. shard-3.gates.example"));
    }
    if value.len() > 128 {
        return Err(Refusal::new(format!(
            "prove: server is {} bytes — 128 is the limit",
            value.len()
        )));
    }
    if value.contains("://") || value.contains('/') {
        return Err(Refusal::new(
            "prove: server is a DOMAIN, not a url — `shard-3.gates.example` or \
             `shard-3.gates.example:7777`, with no scheme and no path. EIP-4361 \
             binds the domain, and the launcher builds the URI line itself.",
        ));
    }
    let (host, port) = match value.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (value, None),
    };
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(Refusal::new(format!(
            "prove: server {value:?} is not a hostname — letters, digits, dots \
             and hyphens, optionally followed by :port"
        )));
    }
    if let Some(p) = port {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return Err(Refusal::new(format!(
                "prove: server {value:?} has a port that is not a number"
            )));
        }
    }
    Ok(())
}

/// The `nonce` a game's server issued.
///
/// ⚠ **EIP-4361 requires at least 8 alphanumeric characters, and that is
/// narrower than the old field allowed** — no `.-_:/+=@` any more. A hex
/// string, a uuid with the dashes stripped, or base62 all fit; a
/// dash-separated uuid does not. The alphabet is what lets an off-the-shelf
/// SIWE parser read the message back, and it is also the injection guard: a
/// value that cannot contain a newline cannot add a line.
///
/// The length floor is the spec's, and it is doing real work — an 8-character
/// alphanumeric nonce is the point below which replay stops being expensive.
pub fn check_nonce(value: &str) -> Result<(), Refusal> {
    if value.is_empty() {
        return Err(Refusal::new("prove: nonce is empty"));
    }
    if value.len() < 8 {
        return Err(Refusal::new(format!(
            "prove: nonce is {} characters — EIP-4361 wants at least 8, so a \
             captured proof is not cheap to guess",
            value.len()
        )));
    }
    if value.len() > 128 {
        return Err(Refusal::new(format!(
            "prove: nonce is {} bytes — 128 is the limit",
            value.len()
        )));
    }
    if let Some(bad) = value.chars().find(|c| !c.is_ascii_alphanumeric()) {
        return Err(Refusal::new(format!(
            "prove: nonce contains {bad:?} — EIP-4361 nonces are alphanumeric \
             only. Hex and stripped uuids fit; dashes and colons do not."
        )));
    }
    Ok(())
}

pub fn sign_family(text: &str) -> Result<String, Refusal> {
    let first = text.lines().next().unwrap_or("");
    let rest = first.strip_prefix("scry ").ok_or_else(|| {
        Refusal::new(
            "a signable message's first line is `scry <family>` \
             (meter/playauth.py builds it)",
        )
    })?;
    let family = rest.trim();
    if family.is_empty() || !family.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return Err(Refusal::new(format!(
            "{family:?} is not a family — lowercase letters and dashes"
        )));
    }
    Ok(family.to_string())
}
