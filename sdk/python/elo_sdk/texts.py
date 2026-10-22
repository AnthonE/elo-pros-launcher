"""The exact texts a wallet signs, rebuilt offline.

A client that signs whatever a server hands it has given that server its
signature. These build the text from what was asked for, so `Client` can
refuse a server that offers anything else. Each is pinned to the meter's own
code by sdk/testdata/ (friends_texts.json, face.json), which
meter/test_sdk_fixtures.py rebuilds on the server side.
"""
from __future__ import annotations

import json
import re
import time
from typing import Optional

from elo_overlay import play_message

FRIENDS_ACTION = "friends"   # meter/friends.py
FACE_ACTION = "profile"      # meter/faces.py

# Each friends op and the one argument it takes, a wallet (friends.OPS).
OPS = {"ask": "to", "accept": "from", "decline": "from", "cancel": "to",
       "remove": "wallet", "block": "wallet", "unblock": "wallet", "invite": "to"}

_WALLET = re.compile(r"^0x[0-9a-fA-F]{40}$")


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
                model: Optional[str] = None) -> str:
    """The signed detail of a face change (`faces._detail`): sorted compact
    JSON of `at` and only the fields being changed. "" clears a field; None
    leaves it as it is and keeps it out of the text."""
    d: dict = {"at": int(at)}
    for k, v in (("name", name), ("picture_sha256", picture_sha256), ("kind", kind),
                 ("harness", harness), ("model", model)):
        if v is not None:
            d[k] = v
    return json.dumps(d, sort_keys=True, separators=(",", ":"))


__all__ = ["FRIENDS_ACTION", "FACE_ACTION", "OPS", "op_key", "is_wallet", "day_of",
           "friends_detail", "friends_sign_text", "face_detail", "play_message"]
