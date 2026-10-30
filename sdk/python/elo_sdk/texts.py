"""The exact texts a wallet signs, rebuilt offline.

A client that signs whatever a server hands it has given that server its
signature. These build the text from what was asked for, so `Client` can
refuse a server that offers anything else. Each is pinned to the meter's own
code by sdk/testdata/ (friends_texts.json, face.json, buddy_texts.json,
board_texts.json, entitle_texts.json), which meter/test_sdk_fixtures.py
rebuilds on the server side.
"""
from __future__ import annotations

import hashlib
import json
import re
import time
from typing import Any, Optional

from elo_overlay import play_message

FRIENDS_ACTION = "friends"   # meter/friends.py
FACE_ACTION = "profile"      # meter/faces.py
BUDDY_ACTION = "buddy"       # meter/buddy.py

# Every op /buddy/prepare takes (buddy.OPS).
BUDDY_OPS = ("trial", "create", "update", "face", "pause", "resume", "retire", "token", "fund", "gift")

# Each friends op and the one argument it takes, a wallet (friends.OPS).
OPS = {"ask": "to", "accept": "from", "decline": "from", "cancel": "to",
       "remove": "wallet", "block": "wallet", "unblock": "wallet", "invite": "to"}

_WALLET = re.compile(r"^0x[0-9a-fA-F]{40}$")
_DAY = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$")
_HEXISH = re.compile(r"^(0x[0-9a-fA-F]{40}|[0-9a-fA-F]{64})$")
_CENTS = re.compile(r"^[0-9]{1,9}(\.[0-9]{1,6})?$")


def op_key(op: str) -> Optional[str]:
    """The argument a friends op takes, or None for an op the server does not know."""
    return OPS.get(op)


def is_wallet(s) -> bool:
    """`0x` and 40 hex characters, either case."""
    return isinstance(s, str) and bool(_WALLET.match(s))


def day_of(at: int) -> str:
    """The UTC day (YYYY-MM-DD) of a unix time (`playauth.day_of`). A text
    keyed to it and signed at 23:59 still verifies at 00:00."""
    return time.strftime("%Y-%m-%d", time.gmtime(int(at)))


def friends_detail(op: str, other: str, at: int) -> Optional[str]:
    """The signed detail of a friends change: sorted compact JSON of its args,
    `at` and op (`friends.detail_of`), the other wallet lowercased. None for an
    op the server does not know or an argument that is not a wallet."""
    key = op_key(op)
    other = (other or "").strip() if isinstance(other, str) else ""
    if key is None or not is_wallet(other):
        return None
    return json.dumps({"args": {key: other.lower()}, "at": int(at), "op": op},
                      sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def friends_sign_text(me: str, op: str, other: str, at: int) -> Optional[str]:
    """The whole text a friends change signs:
    `play_message("friends", me, detail, day_of(at))`, both wallets lowercased."""
    me = (me or "").strip() if isinstance(me, str) else ""
    detail = friends_detail(op, other, at)
    if detail is None or not is_wallet(me):
        return None
    return play_message(FRIENDS_ACTION, me, detail, day_of(at))


def face_detail(at: int, name: Optional[str] = None, picture_sha256: Optional[str] = None,
                kind: Optional[str] = None, harness: Optional[str] = None,
                model: Optional[str] = None, avatar: Optional[int] = None) -> str:
    """The signed detail of a face change (`faces._detail`): sorted compact
    JSON of `at` and only the fields being changed. "" clears a field (0 the
    avatar); None leaves it as it is and keeps it out of the text."""
    d: dict = {"at": int(at)}
    for k, v in (("name", name), ("picture_sha256", picture_sha256), ("kind", kind),
                 ("harness", harness), ("model", model), ("avatar", avatar)):
        if v is not None:
            d[k] = v
    return json.dumps(d, sort_keys=True, separators=(",", ":"))


def read_play(text, action: str, wallet: str) -> Optional[dict]:
    """The `day` and `detail` of an `elo play` text, when it is one for this
    action and wallet and nothing else: five lines, the detail on one. None
    for any other text."""
    if not isinstance(text, str) or not is_wallet((wallet or "").strip()):
        return None
    lines = text.split("\n")
    if len(lines) != 5 or not lines[3].startswith("day: ") or not lines[4].startswith("detail: "):
        return None
    day, detail = lines[3][5:], lines[4][8:]
    if not _DAY.match(day) or text != play_message(action, wallet.strip(), detail, day):
        return None
    return {"day": day, "detail": detail}


def buddy_change(text, wallet: str) -> Optional[dict]:
    """The change a buddy text signs, read back out of it: `day`, `op`,
    `args` and `at` (the detail is buddy.detail_of's JSON). None when the
    text is not a buddy text for this wallet."""
    got = read_play(text, BUDDY_ACTION, wallet)
    if got is None:
        return None
    try:
        d = json.loads(got["detail"])
    except ValueError:
        return None
    if (not isinstance(d, dict) or set(d) != {"args", "at", "op"} or not isinstance(d["op"], str)
            or not isinstance(d["args"], dict) or not isinstance(d["at"], int) or isinstance(d["at"], bool)):
        return None
    return {"day": got["day"], "op": d["op"], "args": d["args"], "at": d["at"]}


def _same(asked: Any, signed: Any) -> bool:
    if isinstance(asked, bool) or isinstance(signed, bool):
        return isinstance(asked, bool) and isinstance(signed, bool) and asked == signed
    if isinstance(signed, (int, float)):
        if isinstance(asked, str) and _CENTS.match(asked.strip()):
            asked = float(asked.strip())
        return isinstance(asked, (int, float)) and abs(asked - signed) <= 0.005 + 1e-9
    if isinstance(signed, str):
        if isinstance(asked, int):
            return str(asked) == signed
        if not isinstance(asked, str):
            return False
        a = " ".join(asked.split())
        return a == signed or (bool(_HEXISH.match(a)) and a.lower() == signed.lower())
    if isinstance(signed, dict):
        return same_args(asked, signed)
    return asked is None and signed is None


def same_args(asked, signed) -> bool:
    """Is the change signed the change asked for? The same keys, and each
    value the same: a string once its whitespace is collapsed (a wallet or a
    sha256 in either case), a dollar amount to the cent, a number given as an
    id the same digits. What the server cleans; nothing it could swap."""
    return (isinstance(asked, dict) and isinstance(signed, dict) and set(asked) == set(signed)
            and all(_same(asked[k], signed[k]) for k in signed))


def claim_text(wallet: str, quest_id: str, day: Optional[str] = None) -> str:
    """The text a board claim signs: the bounty's id is the detail."""
    return play_message("claim", wallet, quest_id, day)


def submit_detail(quest_id: str, url: str, n: int) -> str:
    """A delivery's signed detail (`munus.submit_detail`): the bounty, the
    first 16 hex of the url's sha256, and the wallet's 0-based delivery index
    on that bounty."""
    return f"{quest_id} {hashlib.sha256(url.encode()).hexdigest()[:16]} #{int(n)}"


def submit_text(wallet: str, quest_id: str, url: str, n: int, day: Optional[str] = None) -> str:
    """The text a board delivery signs."""
    return play_message("submit", wallet, submit_detail(quest_id, url, n), day)


def entitle_text(game: str, wallet: str, day: Optional[str] = None) -> str:
    """The text a download grant signs (`tickets.entitle_message`): the game,
    the wallet lowercased, and the UTC day."""
    d = day or time.strftime("%Y-%m-%d", time.gmtime())
    return f"elo entitle\ngame: {game}\nwallet: {wallet.strip().lower()}\nday: {d}"


__all__ = ["FRIENDS_ACTION", "FACE_ACTION", "BUDDY_ACTION", "OPS", "BUDDY_OPS", "op_key", "is_wallet",
           "day_of", "friends_detail", "friends_sign_text", "face_detail", "read_play", "buddy_change",
           "same_args", "claim_text", "submit_detail", "submit_text", "entitle_text", "play_message"]
