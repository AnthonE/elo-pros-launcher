//! The platform's answers as Rust types.
//!
//! Every field is read leniently: a field the server leaves out, sends as
//! null or sends in another type reads as its default (`None`, `false`, `0`,
//! empty) rather than failing the whole answer, and fields this SDK does not
//! know are ignored. So a server that grows a field never breaks a game, and
//! a defaulted field is never mistaken for a reading: optional facts are
//! `Option`s.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// A field's value, or its default when it is missing, null or another type.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

macro_rules! model {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $f:ident : $t:ty,)* }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct $name {
            $($(#[$fm])* #[serde(deserialize_with = "lenient")] pub $f: $t,)*
        }
    };
}

fn short(wallet: &str) -> String {
    let c: Vec<char> = wallet.chars().collect();
    if c.len() >= 10 {
        let head: String = c[..6].iter().collect();
        let tail: String = c[c.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    } else {
        wallet.to_string()
    }
}

// ── friends ─────────────────────────────────────────────────────────────────

model! {
    /// `GET /api/friends/{wallet}`: a wallet's friends with who is playing
    /// what, its buddies, and its pending asks. Public, like playtime.
    pub struct FriendsView {
        pub wallet: String,
        /// How many friends: the book's, buddies not counted.
        pub count: u64,
        /// Online first, then the most recently seen.
        pub friends: Vec<Friend>,
        /// The agents this wallet runs, in the friend row's shape plus `buddy`.
        pub buddies: Vec<Friend>,
        pub asks_in: Vec<Ask>,
        pub asks_out: Vec<Ask>,
        /// Whether presence could be read. When it could not, nobody reads
        /// online, and that is "could not look", not "everyone is offline".
        pub presence: Presence,
        pub limits: Limits,
    }
}

model! {
    /// One wallet on the list: a friend, or a buddy (`buddy` set).
    pub struct Friend {
        pub wallet: String,
        /// When the friendship began, or when the buddy was made (unix seconds).
        pub since: Option<i64>,
        pub name: Option<String>,
        /// A url under `/api/`, relative to the origin.
        pub pic: Option<String>,
        pub agent: bool,
        pub online: bool,
        pub playing: Option<Playing>,
        pub last_online: Option<i64>,
        /// Set on a buddy row: which of the owner's agents this is.
        pub buddy: Option<BuddyTag>,
    }
}

impl Friend {
    /// The name to draw: the face's name, else a short wallet.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| short(&self.wallet))
    }

    pub fn is_buddy(&self) -> bool {
        self.buddy.is_some()
    }
}

model! {
    /// Where a wallet is playing right now.
    pub struct Playing {
        pub game: String,
        pub title: Option<String>,
        /// The shard row id in the title's server list.
        pub server: Option<String>,
        pub server_name: Option<String>,
        pub since: Option<i64>,
        /// `elo://join/<slug>/<host:port>`, which the launcher opens.
        pub join: Option<String>,
        /// The game's rich-presence line ("Wave 5 · 3/4"), when it writes one.
        pub status: Option<String>,
    }
}

model! {
    /// A buddy row's tag: the agent's id and kind (`buddy` or `solo`).
    pub struct BuddyTag {
        pub id: String,
        pub kind: String,
    }
}

model! {
    /// A pending ask, either way.
    pub struct Ask {
        pub wallet: String,
        pub at: Option<i64>,
        pub name: Option<String>,
    }
}

model! {
    pub struct Presence {
        pub reachable: bool,
        pub why: Option<String>,
    }
}

model! {
    pub struct Limits {
        pub friends_max: Option<u64>,
        pub asks_out_max: Option<u64>,
    }
}

model! {
    /// `POST /api/friends/prepare`: the text to sign and its `at`.
    pub struct FriendsPrepared {
        pub sign_this: String,
        pub at: u64,
        pub args: Value,
        pub op: String,
    }
}

model! {
    /// `POST /api/friends/act`: done, with the signer's list after it.
    pub struct FriendsActed {
        pub ok: bool,
        pub op: String,
        pub view: Option<FriendsView>,
    }
}

// ── tickets ─────────────────────────────────────────────────────────────────

model! {
    /// `GET /api/ticket/{slug}`: a title's copy, its prices and how to buy.
    /// A title with no ticket contract is free (`ticketed` false).
    pub struct Ticket {
        pub game: String,
        pub ticketed: bool,
        pub contract: Option<String>,
        pub reachable: Option<bool>,
        /// `house` or `community`.
        pub lane: Option<String>,
        pub price_usd: Option<f64>,
        /// By rail name (`eth`, `elo`, `usdg`). A rail at 0 is shut.
        pub rails: BTreeMap<String, Rail>,
        /// Every rail the contract has, in its own order.
        pub all_rails: Vec<ChainRail>,
        pub coins: BTreeMap<String, Option<String>>,
        pub copies_minted: Option<u64>,
        /// Copies paid for; null until every token's rail has been read.
        pub copies_sold: Option<u64>,
        pub free_door: Option<String>,
        pub free_mint: Option<FreeMint>,
        pub how_to_buy: BTreeMap<String, String>,
        pub downloads_gated: bool,
        pub download: Option<String>,
        /// Set on a free title: what that means.
        pub what: Option<String>,
        pub scope: Option<String>,
    }
}

model! {
    pub struct Rail {
        pub amount_wei: Option<String>,
        pub amount_units: Option<String>,
        pub open: bool,
        /// The posted amount at the live price, as a multiple of the dollar price.
        pub usd_now: Option<f64>,
        pub where_it_goes: Option<String>,
    }
}

model! {
    pub struct ChainRail {
        pub rail_id: u64,
        pub label: Option<String>,
        /// `native`, `erc20`, `erc721` or `erc1155`.
        pub kind: Option<String>,
        pub token: Option<String>,
        /// Base units, as a string: wei amounts do not fit a JSON number.
        pub amount: Option<String>,
        pub sink: Option<String>,
        pub open: bool,
        pub payment: bool,
        pub decimals: Option<u32>,
        pub buyable: bool,
    }
}

model! {
    pub struct FreeMint {
        pub open: bool,
        pub per_wallet: Option<u64>,
        /// The house pays the gas.
        pub sponsored: bool,
        pub how: Option<String>,
    }
}

model! {
    /// `GET /api/ticket/{slug}/of/{wallet}`: does this wallet hold a copy.
    pub struct TicketOf {
        pub game: String,
        pub ticketed: Option<bool>,
        pub wallet: Option<String>,
        pub copies: Option<u64>,
        /// None is a failed read, never "does not own".
        pub entitled: Option<bool>,
        pub reachable: Option<bool>,
        pub free_left: Option<u64>,
        pub why: Option<String>,
    }
}

/// One roster sweep ([`crate::Client::check_roster`]): each wallet — 0x
/// lowercased, or the Solana address as given — to `Some(true)`,
/// `Some(false)` or `None`. Kick on `Some(false)` only: `None` is a read that
/// failed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Roster {
    pub game: String,
    /// False for a free title: every wallet is entitled.
    pub ticketed: bool,
    pub entitled: BTreeMap<String, Option<bool>>,
}

// ── sign-in ─────────────────────────────────────────────────────────────────

model! {
    /// `POST /api/signin/check`: a sign-in found fresh, for your domain and
    /// nonce, and signed by the wallet it names — a plain key, a smart wallet
    /// or a Solana key.
    pub struct SignIn {
        pub ok: bool,
        /// EIP-55 for an Ethereum wallet, base58 for a Solana one
        /// (case-sensitive: store it as given).
        pub address: String,
        /// `evm` or `solana`.
        pub chain: String,
        /// `4663` for Robinhood Chain; `mainnet` for a Solana sign-in.
        pub chain_id: String,
        pub domain: String,
        pub uri: String,
        pub nonce: String,
        pub issued_at: String,
        pub expiration_time: Option<String>,
        pub statement: Option<String>,
    }
}

// ── items ───────────────────────────────────────────────────────────────────

model! {
    /// `GET /api/items/of/{wallet}`: what the wallet holds, each item re-read
    /// on chain. `configured: Some(false)` is a meter with no item contract.
    pub struct Items {
        pub wallet: Option<String>,
        pub collection: Option<String>,
        pub title: Option<String>,
        pub items: Vec<Item>,
        pub count: u64,
        /// An eth_call, so authoritative over the index's count.
        pub balance_onchain: Option<u64>,
        pub index_disagrees: bool,
        pub truncated: Option<u64>,
        pub cap: Option<u64>,
        pub vault_errors: Vec<String>,
        pub configured: Option<bool>,
        pub reachable: Option<bool>,
        pub error: Option<String>,
        pub why: Option<String>,
    }
}

model! {
    /// One held item: a token (`token_id`), or a skin held as its vault's
    /// token (`via` = `vault`, no `token_id`).
    pub struct Item {
        pub token_id: Option<u64>,
        pub catalog_id: Option<u64>,
        pub mint_kind: Option<u64>,
        pub mint_kind_name: Option<String>,
        pub mint_ref: Option<String>,
        /// The attributes derived from `mint_ref` (`/api/items/provenance`).
        pub derived: Option<Value>,
        pub catalog_name: Option<String>,
        pub shape: Option<String>,
        pub image: Option<String>,
        pub texture_url: Option<String>,
        pub via: Option<String>,
        pub vault: Option<String>,
        pub copies: Option<u64>,
        pub balance: Option<String>,
        pub error: Option<String>,
    }
}

impl Item {
    /// Held as a vault's token rather than as an item token.
    pub fn is_vault(&self) -> bool {
        self.via.as_deref() == Some("vault")
    }
}

// ── faces and profiles ──────────────────────────────────────────────────────

model! {
    /// `GET /api/face/{wallet}`: the name and picture a wallet signed, and
    /// what it says it is. Every field is a claim the wallet made.
    pub struct Face {
        pub wallet: String,
        pub name: Option<String>,
        /// A url under `/api/`, relative to the origin.
        pub picture: Option<String>,
        pub pic: Option<String>,
        /// `agent` or `human`, self-declared and signed.
        pub kind: Option<String>,
        pub harness: Option<String>,
        pub model: Option<String>,
        pub updated: Option<i64>,
        /// The guild tag this wallet shows, the same badge as
        /// [`crate::Client::guild_tags`]; `None` when it shows none.
        pub guild: Option<GuildBadge>,
    }
}

// ── guilds ──────────────────────────────────────────────────────────────────

model! {
    /// The guild tag a game draws beside a player's name: one guild's tag,
    /// name and emblem, and the player's role in it.
    pub struct GuildBadge {
        /// 2 to 5 letters or digits, like `OWL`.
        pub tag: String,
        pub name: Option<String>,
        /// The emblem's colour: lime, ember, sky, violet, gold, rose, slate, teal.
        pub color: Option<String>,
        /// The emblem's shape: shield, round, diamond, banner, hex.
        pub shape: Option<String>,
        /// `leader`, `officer` or `member`.
        pub role: Option<String>,
        /// The guild's page, relative to the origin (`/g/OWL`).
        pub page: Option<String>,
    }
}

model! {
    /// `GET /api/guilds/tags?wallets=`: each wallet asked, lowercased, mapped
    /// to the tag it shows, or `None` for a wallet that shows none.
    pub struct GuildTags {
        pub tags: BTreeMap<String, Option<GuildBadge>>,
        pub how: Option<String>,
    }
}

impl GuildTags {
    /// The badge `wallet` shows, if any (any case).
    pub fn of(&self, wallet: &str) -> Option<&GuildBadge> {
        self.tags
            .get(&wallet.trim().to_ascii_lowercase())
            .and_then(Option::as_ref)
    }
}

model! {
    /// A guild's emblem: a shape and a colour, drawn by the client.
    pub struct GuildEmblem {
        pub shape: Option<String>,
        pub color: Option<String>,
    }
}

model! {
    /// One guild a wallet is in, asked to, or invited to.
    pub struct GuildCard {
        pub tag: String,
        pub name: Option<String>,
        pub line: Option<String>,
        pub emblem: GuildEmblem,
        /// `open`, `apply` or `invite`.
        pub door: Option<String>,
        pub founded: Option<i64>,
        pub members: u64,
        pub officers: u64,
        pub page: Option<String>,
        /// This wallet's role, on a guild it is in.
        pub role: Option<String>,
        /// When it joined, on a guild it is in.
        pub since: Option<i64>,
        /// When it asked or was invited, on an ask or an invite.
        pub at: Option<i64>,
    }
}

model! {
    /// `GET /api/guilds/of/{wallet}`: the guilds a wallet is in, its asks,
    /// the invites waiting on it, and the tag it shows.
    pub struct GuildsOf {
        pub wallet: String,
        pub guilds: Vec<GuildCard>,
        pub applied: Vec<GuildCard>,
        pub invites: Vec<GuildCard>,
        /// The tag games show over this wallet, if any.
        pub worn: Option<String>,
        /// It chose to show no tag.
        pub hidden: bool,
    }
}

model! {
    /// `POST /api/face/prepare`.
    pub struct FacePrepared {
        pub sign_this: String,
        pub at: u64,
        pub detail: String,
    }
}

model! {
    /// `GET /api/who?handle=`: one party by wallet, npub or name.
    pub struct Who {
        pub handle: Option<String>,
        /// `wallet`, `npub` or `name`.
        pub handle_kind: Option<String>,
        pub matches: u64,
        pub authoritative: bool,
        pub found: Option<WhoFound>,
        pub note: Option<String>,
    }
}

/// The identity `/who` found. The named fields are the common ones; the rest
/// of what the server sent is in `rest`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhoFound {
    #[serde(deserialize_with = "lenient")]
    pub wallet: Option<String>,
    /// The name `/who` picked: the wallet's own name, else its hive handle.
    #[serde(deserialize_with = "lenient")]
    pub agent: Option<String>,
    /// The hive handle as `name@host`, held by this wallet, if any.
    #[serde(deserialize_with = "lenient")]
    pub nip05: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub avatar: Option<String>,
    /// The wallet's own name and picture.
    #[serde(deserialize_with = "lenient")]
    pub profile: Option<Face>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, Value>,
}

// ── leaderboards ────────────────────────────────────────────────────────────

model! {
    /// `GET /api/leaderboard/{slug}`: a title's boards, newest wipe first,
    /// ranked by a game score the shard reported.
    pub struct Leaderboard {
        pub slug: String,
        pub reachable: bool,
        pub published: bool,
        pub boards: Vec<Board>,
        pub cycles: u64,
        pub from_play: u64,
        pub from_examples: u64,
        pub example_only: bool,
        pub note: Option<String>,
        pub rule: Option<String>,
        /// `agent` or `human` when the board was asked for one side.
        pub kind: Option<String>,
        pub url: Option<String>,
        pub why: Option<String>,
        pub shard: Option<ShardReport>,
    }
}

model! {
    pub struct Board {
        pub name: Option<String>,
        pub wipe: Option<u64>,
        /// A closed wipe's board.
        #[serde(rename = "final")]
        pub is_final: bool,
        /// `shard` or `file`.
        pub source: Option<String>,
        pub cycle_id: Option<Value>,
        pub segment_id: Option<String>,
        pub seq: Option<i64>,
        /// The entry field the board ranks on (`island`, `works`, …).
        pub sort_key: String,
        pub ranked: Option<u64>,
        pub example_only: bool,
        pub grounded: bool,
        pub grounding: Option<String>,
        pub refused: Option<String>,
        pub count: u64,
        pub entries: Vec<Entry>,
    }
}

impl Board {
    /// An entry's score on this board.
    pub fn score_of(&self, e: &Entry) -> Option<f64> {
        e.score(&self.sort_key)
    }
}

/// One ranked row. The score sits under the board's `sort_key`, in `rest`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    #[serde(deserialize_with = "lenient")]
    pub rank: u64,
    #[serde(deserialize_with = "lenient")]
    pub wallet: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub agent: Option<bool>,
    /// Where the shard's own board put the row.
    #[serde(deserialize_with = "lenient")]
    pub place: Option<u64>,
    #[serde(deserialize_with = "lenient")]
    pub harness: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub model: Option<String>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, Value>,
}

impl Entry {
    /// The value under `key`, as a number.
    pub fn score(&self, key: &str) -> Option<f64> {
        self.rest.get(key).and_then(Value::as_f64)
    }
}

model! {
    pub struct ShardReport {
        pub reachable: bool,
        pub why: Option<String>,
        pub server: Option<String>,
        pub wipe: Option<u64>,
        pub last_wipe: Option<u64>,
        pub url: Option<String>,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_missing_and_mistyped_fields_read_as_defaults() {
        let f: Friend = serde_json::from_str(
            r#"{"wallet":"0xab","since":null,"online":"yes","playing":7,"extra":{"x":1}}"#,
        )
        .unwrap();
        assert_eq!(f.wallet, "0xab");
        assert_eq!(f.since, None);
        assert!(!f.online);
        assert_eq!(f.playing, None);
        assert_eq!(f.label(), "0xab");
    }
}
