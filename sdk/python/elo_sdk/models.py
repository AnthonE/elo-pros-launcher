"""The platform's answers as dataclasses.

Every field is read leniently: one the server leaves out, sends as null or
sends in another type reads as its default (None, False, 0, empty) rather
than failing the whole answer, and fields this SDK does not know are kept in
`raw`. Optional facts are None, never a made-up zero.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Callable, Dict, List, Optional


def _s(v) -> Optional[str]:
    return v if isinstance(v, str) else None


def _i(v) -> Optional[int]:
    return v if isinstance(v, int) and not isinstance(v, bool) else None


def _n(v) -> Optional[float]:
    return float(v) if isinstance(v, (int, float)) and not isinstance(v, bool) else None


def _b(v) -> bool:
    return v is True


def _ob(v) -> Optional[bool]:
    return v if isinstance(v, bool) else None


def _d(v) -> dict:
    return v if isinstance(v, dict) else {}


def _l(v, conv: Callable[[Any], Any]) -> list:
    return [conv(x) for x in v if isinstance(x, dict)] if isinstance(v, list) else []


def _short(wallet: str) -> str:
    return f"{wallet[:6]}…{wallet[-4:]}" if len(wallet) >= 10 else wallet


# ── friends ─────────────────────────────────────────────────────────────────
@dataclass
class Playing:
    """Where a wallet is playing right now. `join` is `elo://join/<slug>/<host:port>`."""
    game: str = ""
    title: Optional[str] = None
    server: Optional[str] = None
    server_name: Optional[str] = None
    since: Optional[int] = None
    join: Optional[str] = None

    @classmethod
    def from_json(cls, d) -> "Playing":
        d = _d(d)
        return cls(game=_s(d.get("game")) or "", title=_s(d.get("title")), server=_s(d.get("server")),
                   server_name=_s(d.get("server_name")), since=_i(d.get("since")), join=_s(d.get("join")))


@dataclass
class BuddyTag:
    """A buddy row's tag: the agent's id and kind (`buddy` or `solo`)."""
    id: str = ""
    kind: str = ""


@dataclass
class Friend:
    """One wallet on the list: a friend, or a buddy (`buddy` set)."""
    wallet: str = ""
    since: Optional[int] = None
    name: Optional[str] = None
    pic: Optional[str] = None
    agent: bool = False
    online: bool = False
    playing: Optional[Playing] = None
    last_online: Optional[int] = None
    buddy: Optional[BuddyTag] = None
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "Friend":
        d = _d(d)
        b = d.get("buddy")
        return cls(wallet=_s(d.get("wallet")) or "", since=_i(d.get("since")), name=_s(d.get("name")),
                   pic=_s(d.get("pic")), agent=_b(d.get("agent")), online=_b(d.get("online")),
                   playing=Playing.from_json(d["playing"]) if isinstance(d.get("playing"), dict) else None,
                   last_online=_i(d.get("last_online")),
                   buddy=BuddyTag(_s(b.get("id")) or "", _s(b.get("kind")) or "") if isinstance(b, dict) else None,
                   raw=d)

    @property
    def label(self) -> str:
        """The name to draw: the face's name, else a short wallet."""
        return self.name or _short(self.wallet)

    @property
    def is_buddy(self) -> bool:
        return self.buddy is not None


@dataclass
class Ask:
    wallet: str = ""
    at: Optional[int] = None
    name: Optional[str] = None

    @classmethod
    def from_json(cls, d) -> "Ask":
        d = _d(d)
        return cls(_s(d.get("wallet")) or "", _i(d.get("at")), _s(d.get("name")))


@dataclass
class Presence:
    """Whether presence could be read. When it could not, nobody reads
    online, and that is "could not look", not "everyone is offline"."""
    reachable: bool = False
    why: Optional[str] = None


@dataclass
class Limits:
    friends_max: Optional[int] = None
    asks_out_max: Optional[int] = None


@dataclass
class FriendsView:
    """GET /api/friends/{wallet}: friends with who is playing what, buddies,
    and pending asks. `count` is the friends alone, buddies not counted."""
    wallet: str = ""
    count: int = 0
    friends: List[Friend] = field(default_factory=list)
    buddies: List[Friend] = field(default_factory=list)
    asks_in: List[Ask] = field(default_factory=list)
    asks_out: List[Ask] = field(default_factory=list)
    presence: Presence = field(default_factory=Presence)
    limits: Limits = field(default_factory=Limits)
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "FriendsView":
        d = _d(d)
        p, lim = _d(d.get("presence")), _d(d.get("limits"))
        return cls(wallet=_s(d.get("wallet")) or "", count=_i(d.get("count")) or 0,
                   friends=_l(d.get("friends"), Friend.from_json),
                   buddies=_l(d.get("buddies"), Friend.from_json),
                   asks_in=_l(d.get("asks_in"), Ask.from_json), asks_out=_l(d.get("asks_out"), Ask.from_json),
                   presence=Presence(_b(p.get("reachable")), _s(p.get("why"))),
                   limits=Limits(_i(lim.get("friends_max")), _i(lim.get("asks_out_max"))), raw=d)


@dataclass
class FriendsActed:
    """POST /api/friends/act: done, with the signer's list after it."""
    ok: bool = False
    op: str = ""
    view: Optional[FriendsView] = None

    @classmethod
    def from_json(cls, d) -> "FriendsActed":
        d = _d(d)
        return cls(_b(d.get("ok")), _s(d.get("op")) or "",
                   FriendsView.from_json(d["view"]) if isinstance(d.get("view"), dict) else None)


# ── tickets ─────────────────────────────────────────────────────────────────
@dataclass
class Rail:
    amount_wei: Optional[str] = None
    amount_units: Optional[str] = None
    open: bool = False
    usd_now: Optional[float] = None
    where_it_goes: Optional[str] = None

    @classmethod
    def from_json(cls, d) -> "Rail":
        d = _d(d)
        return cls(_s(d.get("amount_wei")), _s(d.get("amount_units")), _b(d.get("open")),
                   _n(d.get("usd_now")), _s(d.get("where_it_goes")))


@dataclass
class ChainRail:
    rail_id: int = 0
    label: Optional[str] = None
    kind: Optional[str] = None
    token: Optional[str] = None
    amount: Optional[str] = None     # base units as a string: wei does not fit a float
    sink: Optional[str] = None
    open: bool = False
    payment: bool = False
    decimals: Optional[int] = None
    buyable: bool = False

    @classmethod
    def from_json(cls, d) -> "ChainRail":
        d = _d(d)
        return cls(_i(d.get("rail_id")) or 0, _s(d.get("label")), _s(d.get("kind")), _s(d.get("token")),
                   _s(d.get("amount")), _s(d.get("sink")), _b(d.get("open")), _b(d.get("payment")),
                   _i(d.get("decimals")), _b(d.get("buyable")))


@dataclass
class FreeMint:
    open: bool = False
    per_wallet: Optional[int] = None
    sponsored: bool = False
    how: Optional[str] = None


@dataclass
class Ticket:
    """GET /api/ticket/{slug}: a title's copy, its prices and how to buy. A
    title with no ticket contract is free (`ticketed` False). A rail at 0 is shut."""
    game: str = ""
    ticketed: bool = False
    contract: Optional[str] = None
    reachable: Optional[bool] = None
    lane: Optional[str] = None
    price_usd: Optional[float] = None
    rails: Dict[str, Rail] = field(default_factory=dict)
    all_rails: List[ChainRail] = field(default_factory=list)
    copies_minted: Optional[int] = None
    copies_sold: Optional[int] = None
    free_mint: Optional[FreeMint] = None
    how_to_buy: Dict[str, str] = field(default_factory=dict)
    downloads_gated: bool = False
    what: Optional[str] = None
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "Ticket":
        d = _d(d)
        fm = d.get("free_mint")
        return cls(game=_s(d.get("game")) or "", ticketed=_b(d.get("ticketed")), contract=_s(d.get("contract")),
                   reachable=_ob(d.get("reachable")), lane=_s(d.get("lane")), price_usd=_n(d.get("price_usd")),
                   rails={k: Rail.from_json(v) for k, v in _d(d.get("rails")).items() if isinstance(v, dict)},
                   all_rails=_l(d.get("all_rails"), ChainRail.from_json),
                   copies_minted=_i(d.get("copies_minted")), copies_sold=_i(d.get("copies_sold")),
                   free_mint=(FreeMint(_b(fm.get("open")), _i(fm.get("per_wallet")), _b(fm.get("sponsored")),
                                       _s(fm.get("how"))) if isinstance(fm, dict) else None),
                   how_to_buy={k: v for k, v in _d(d.get("how_to_buy")).items() if isinstance(v, str)},
                   downloads_gated=_b(d.get("downloads_gated")), what=_s(d.get("what")), raw=d)


@dataclass
class TicketOf:
    """GET /api/ticket/{slug}/of/{wallet}. `entitled` None is a failed read,
    never "does not own"."""
    game: str = ""
    ticketed: Optional[bool] = None
    wallet: Optional[str] = None
    copies: Optional[int] = None
    entitled: Optional[bool] = None
    reachable: Optional[bool] = None
    free_left: Optional[int] = None
    why: Optional[str] = None

    @classmethod
    def from_json(cls, d) -> "TicketOf":
        d = _d(d)
        return cls(_s(d.get("game")) or "", _ob(d.get("ticketed")), _s(d.get("wallet")), _i(d.get("copies")),
                   _ob(d.get("entitled")), _ob(d.get("reachable")), _i(d.get("free_left")), _s(d.get("why")))


# ── items ───────────────────────────────────────────────────────────────────
@dataclass
class Item:
    """A held item: a token (`token_id`), or a skin held as its vault's token
    (`via` == "vault", no `token_id`)."""
    token_id: Optional[int] = None
    catalog_id: Optional[int] = None
    mint_kind_name: Optional[str] = None
    mint_ref: Optional[str] = None
    derived: Optional[dict] = None
    catalog_name: Optional[str] = None
    shape: Optional[str] = None
    image: Optional[str] = None
    via: Optional[str] = None
    vault: Optional[str] = None
    copies: Optional[int] = None
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "Item":
        d = _d(d)
        der = d.get("derived")
        return cls(_i(d.get("token_id")), _i(d.get("catalog_id")), _s(d.get("mint_kind_name")),
                   _s(d.get("mint_ref")), der if isinstance(der, dict) else None, _s(d.get("catalog_name")),
                   _s(d.get("shape")), _s(d.get("image")), _s(d.get("via")), _s(d.get("vault")),
                   _i(d.get("copies")), d)

    @property
    def is_vault(self) -> bool:
        return self.via == "vault"


@dataclass
class Items:
    """GET /api/items/of/{wallet}. `configured` False is a meter with no item contract."""
    wallet: Optional[str] = None
    collection: Optional[str] = None
    title: Optional[str] = None
    items: List[Item] = field(default_factory=list)
    count: int = 0
    balance_onchain: Optional[int] = None
    index_disagrees: bool = False
    truncated: Optional[int] = None
    configured: Optional[bool] = None
    vault_errors: List[str] = field(default_factory=list)
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "Items":
        d = _d(d)
        ve = d.get("vault_errors")
        return cls(_s(d.get("wallet")), _s(d.get("collection")), _s(d.get("title")),
                   _l(d.get("items"), Item.from_json), _i(d.get("count")) or 0, _i(d.get("balance_onchain")),
                   _b(d.get("index_disagrees")), _i(d.get("truncated")), _ob(d.get("configured")),
                   [x for x in ve if isinstance(x, str)] if isinstance(ve, list) else [], d)


# ── faces and profiles ──────────────────────────────────────────────────────
@dataclass
class Face:
    """GET /api/face/{wallet}. Every field is a claim the wallet signed."""
    wallet: str = ""
    name: Optional[str] = None
    picture: Optional[str] = None
    pic: Optional[str] = None
    kind: Optional[str] = None
    harness: Optional[str] = None
    model: Optional[str] = None
    updated: Optional[int] = None

    @classmethod
    def from_json(cls, d) -> "Face":
        d = _d(d)
        return cls(_s(d.get("wallet")) or "", _s(d.get("name")), _s(d.get("picture")), _s(d.get("pic")),
                   _s(d.get("kind")), _s(d.get("harness")), _s(d.get("model")), _i(d.get("updated")))


@dataclass
class WhoFound:
    wallet: Optional[str] = None
    agent: Optional[str] = None       # the sworn name on the register
    vow_id: Optional[str] = None
    sworn: Optional[bool] = None
    avatar: Optional[str] = None
    profile: Optional[Face] = None    # the wallet's own face, when nothing is sworn
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "WhoFound":
        d = _d(d)
        p = d.get("profile")
        return cls(_s(d.get("wallet")), _s(d.get("agent")), _s(d.get("vow_id")), _ob(d.get("sworn")),
                   _s(d.get("avatar")), Face.from_json(p) if isinstance(p, dict) else None, d)


@dataclass
class Who:
    """GET /api/who?handle=: one party by sworn name, npub, vow id or wallet."""
    handle: Optional[str] = None
    handle_kind: Optional[str] = None
    matches: int = 0
    authoritative: bool = False
    found: Optional[WhoFound] = None
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "Who":
        d = _d(d)
        f = d.get("found")
        return cls(_s(d.get("handle")), _s(d.get("handle_kind")), _i(d.get("matches")) or 0,
                   _b(d.get("authoritative")), WhoFound.from_json(f) if isinstance(f, dict) else None, d)


# ── leaderboards ────────────────────────────────────────────────────────────
@dataclass
class Entry:
    """One ranked row. Its score sits under the board's `sort_key` (`score`)."""
    rank: int = 0
    wallet: Optional[str] = None
    name: Optional[str] = None
    agent: Optional[bool] = None
    place: Optional[int] = None
    harness: Optional[str] = None
    model: Optional[str] = None
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "Entry":
        d = _d(d)
        return cls(_i(d.get("rank")) or 0, _s(d.get("wallet")), _s(d.get("name")), _ob(d.get("agent")),
                   _i(d.get("place")), _s(d.get("harness")), _s(d.get("model")), d)

    def score(self, key: str) -> Optional[float]:
        return _n(self.raw.get(key))


@dataclass
class Board:
    name: Optional[str] = None
    wipe: Optional[int] = None
    is_final: bool = False            # `final` on the wire: a closed wipe's board
    source: Optional[str] = None
    sort_key: str = ""
    ranked: Optional[int] = None
    grounded: bool = False
    refused: Optional[str] = None
    count: int = 0
    entries: List[Entry] = field(default_factory=list)
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "Board":
        d = _d(d)
        return cls(_s(d.get("name")), _i(d.get("wipe")), _b(d.get("final")), _s(d.get("source")),
                   _s(d.get("sort_key")) or "", _i(d.get("ranked")), _b(d.get("grounded")), _s(d.get("refused")),
                   _i(d.get("count")) or 0, _l(d.get("entries"), Entry.from_json), d)

    def score_of(self, e: Entry) -> Optional[float]:
        return e.score(self.sort_key)


@dataclass
class Leaderboard:
    """GET /api/leaderboard/{slug}: newest wipe first, ranked by a game score
    the shard reported."""
    slug: str = ""
    reachable: bool = False
    published: bool = False
    boards: List[Board] = field(default_factory=list)
    example_only: bool = False
    kind: Optional[str] = None
    why: Optional[str] = None
    shard: Optional[dict] = None
    raw: dict = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, d) -> "Leaderboard":
        d = _d(d)
        sh = d.get("shard")
        return cls(_s(d.get("slug")) or "", _b(d.get("reachable")), _b(d.get("published")),
                   _l(d.get("boards"), Board.from_json), _b(d.get("example_only")), _s(d.get("kind")),
                   _s(d.get("why")), sh if isinstance(sh, dict) else None, d)


__all__ = ["Playing", "BuddyTag", "Friend", "Ask", "Presence", "Limits", "FriendsView", "FriendsActed",
           "Rail", "ChainRail", "FreeMint", "Ticket", "TicketOf", "Item", "Items", "Face", "WhoFound", "Who",
           "Entry", "Board", "Leaderboard"]
