"""Moving a connected player from one of your servers to another.

The pass gets a player in; a handoff moves them (`docs/client/HANDOFF.md`).
The server a player is on signs a handoff with its server key and hands it
to the client with the address to go to; the client joins that server with
it; that server checks it offline against your game's public keys and admits
the player it names. No call to Elo Pros per hop, and no prompt.

On the server they leave::

    from elo_sdk import ServerKey
    key = ServerKey.from_file("my-game", "zone-1", "/etc/my-game/server.key")
    token = key.handoff(player, "zone1.my-game.example:7000", "zone2.my-game.example:7000",
                        carry=b'{"x":10,"z":-4}')
    # send the client: go to zone2.my-game.example:7000 with token (and the carry)

On the server they arrive at::

    from elo_sdk import Client, HandoffDoor, HandoffRefused
    door = HandoffDoor("my-game", ["zone2.my-game.example:7000"], client=Client())
    try:
        got = door.admit(token, carry)
        player = got.player            # the identity the last server admitted
    except HandoffRefused as r:
        ...                            # r.code: why; let them join the usual way

`sdk/testdata/handoff.json` pins the text, the token and every refusal.
"""
from __future__ import annotations

import base64
import hashlib
import re
import secrets
import threading
import time
from typing import Callable, Dict, Iterable, NamedTuple, Optional

TTL_DEFAULT_S = 60
TTL_MAX_S = 120
SKEW_S = 30
CARRY_MAX = 4096
TOKEN_MAX = 1024
KEYS_REFRESH_S = 300
KEYS_RETRY_S = 60
SEEN_MAX = 200_000

_SLUG = r"[a-z0-9][a-z0-9_-]{0,62}"
_KID = r"[a-z0-9][a-z0-9-]{0,30}"
_PLAYER = r"0x[0-9a-f]{40}|[1-9A-HJ-NP-Za-km-z]{32,44}|acct_[0-9a-f]{20}"
_ADDR = r"(?:[a-z0-9.-]{1,100}|\[[0-9a-f:]{2,39}\]):[0-9]{1,5}"
_ID = r"[A-Za-z0-9_-]{16,64}"
_TEXT = re.compile(
    rf"elo handoff\ngame: ({_SLUG})\nkey: ({_KID})\nplayer: ({_PLAYER})\n"
    rf"from: ({_ADDR})\nto: ({_ADDR})\nid: ({_ID})\nexp: ([0-9]{{1,12}})\ncarry: ([0-9a-f]{{64}}|none)")
_PLAYER_RE = re.compile(_PLAYER)
_ADDR_RE = re.compile(_ADDR)
_ID_ABC = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"

CODES = ("malformed", "wrong_game", "wrong_destination", "expired", "too_long",
         "unknown_key", "bad_signature", "carry", "used", "no_keys")


class HandoffRefused(Exception):
    """Why a handoff was not admitted. `code` is one of CODES; `why` is for a log."""

    def __init__(self, code: str, why: str):
        super().__init__(f"{code}: {why}")
        self.code, self.why = code, why


class Handoff(NamedTuple):
    game: str
    key: str
    player: str
    frm: str
    to: str
    id: str
    exp: int
    carry: Optional[str]


def handoff_text(game: str, key_id: str, player: str, frm: str, to: str, hid: str, exp: int,
                 carry: Optional[bytes] = None) -> str:
    """The exact text a server signs for one handoff."""
    c = hashlib.sha256(carry).hexdigest() if carry else "none"
    return (f"elo handoff\ngame: {game}\nkey: {key_id}\nplayer: {player}\nfrom: {frm}\n"
            f"to: {to}\nid: {hid}\nexp: {int(exp)}\ncarry: {c}")


def _b64u(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode("ascii")


def _unb64u(s: str) -> bytes:
    if not re.fullmatch(r"[A-Za-z0-9_-]*", s):
        raise ValueError("not base64url")
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))


def new_handoff_id(n: int = 22) -> str:
    return "".join(secrets.choice(_ID_ABC) for _ in range(n))


def issue(sign: Callable[[bytes], bytes], game: str, key_id: str, player: str, frm: str, to: str, *,
          carry: Optional[bytes] = None, ttl: int = TTL_DEFAULT_S, now: Optional[int] = None,
          hid: Optional[str] = None) -> str:
    """A handoff token, signed by `sign` (raw Ed25519 over the text's bytes)."""
    if not isinstance(player, str) or not _PLAYER_RE.fullmatch(player):
        raise ValueError("player is a 0x wallet (lowercase), a Solana address or an acct_ id")
    for name, a in (("frm", frm), ("to", to)):
        if not isinstance(a, str) or not _ADDR_RE.fullmatch(a):
            raise ValueError(f"{name} is a server address, host:port, lowercase")
    if isinstance(ttl, bool) or not isinstance(ttl, int) or not 1 <= ttl <= TTL_MAX_S:
        raise ValueError(f"ttl is 1 to {TTL_MAX_S} seconds")
    if carry is not None and len(carry) > CARRY_MAX:
        raise ValueError(f"a handoff carries at most {CARRY_MAX} bytes")
    hid = new_handoff_id() if hid is None else hid
    if not re.fullmatch(_ID, hid):
        raise ValueError("id is 16 to 64 of A-Z a-z 0-9 - _")
    exp = (int(time.time()) if now is None else int(now)) + ttl
    text = handoff_text(game, key_id, player, frm, to, hid, exp, carry).encode()
    return _b64u(text) + "." + _b64u(sign(text))


def read_handoff(token: str) -> Handoff:
    """What a token says, shape only (nothing verified). Raises HandoffRefused."""
    if not isinstance(token, str) or len(token) > TOKEN_MAX or token.count(".") != 1:
        raise HandoffRefused("malformed", "a handoff is base64url(text).base64url(signature)")
    a, b = token.split(".")
    try:
        raw, sig = _unb64u(a), _unb64u(b)
    except ValueError:
        raise HandoffRefused("malformed", "a half is not base64url") from None
    m = _TEXT.fullmatch(raw.decode("utf-8", "replace"))
    if not m or len(sig) != 64:
        raise HandoffRefused("malformed", "the text is not the nine handoff lines")
    g = m.groups()
    return Handoff(g[0], g[1], g[2], g[3], g[4], g[5], int(g[6]), None if g[7] == "none" else g[7])


def _parts(token: str):
    a, b = token.split(".")
    return _unb64u(a), _unb64u(b)


def _verify(pub: bytes, sig: bytes, msg: bytes) -> bool:
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    except ImportError as e:  # pragma: no cover
        raise ImportError('checking a handoff needs the cryptography package — the "server" extra '
                          'of elo-sdk, or pip install cryptography') from e
    try:
        Ed25519PublicKey.from_public_bytes(pub).verify(sig, msg)
        return True
    except (InvalidSignature, ValueError):
        return False


def verify_handoff(token: str, *, game: str, names: Iterable[str], keys: Dict[str, bytes],
                   now: Optional[int] = None, carry: bytes = b"") -> Handoff:
    """Every check but `used`, in the posted order. `keys` maps key id → the
    32-byte public key. A server answering to several addresses names them all."""
    now = int(time.time()) if now is None else int(now)
    h = read_handoff(token)
    if h.game != game:
        raise HandoffRefused("wrong_game", f"this handoff is for {h.game}")
    if h.to not in {str(n).lower() for n in names}:
        raise HandoffRefused("wrong_destination", f"this handoff is for {h.to}")
    if h.exp <= now - SKEW_S:
        raise HandoffRefused("expired", "this handoff has run out")
    if h.exp > now + TTL_MAX_S + SKEW_S:
        raise HandoffRefused("too_long", f"a handoff lives at most {TTL_MAX_S} seconds")
    pub = keys.get(h.key)
    if pub is None:
        raise HandoffRefused("unknown_key", f"{game} has no server key {h.key}")
    raw, sig = _parts(token)
    if not _verify(pub, sig, raw):
        raise HandoffRefused("bad_signature", "the signature is not that key's")
    carry = carry or b""
    if len(carry) > CARRY_MAX:
        raise HandoffRefused("carry", f"a handoff carries at most {CARRY_MAX} bytes")
    if (hashlib.sha256(carry).hexdigest() if carry else None) != h.carry:
        raise HandoffRefused("carry", "the carried bytes are not the ones this handoff names")
    return h


class MemorySeen:
    """The ids this process admitted, until each runs out. Several processes
    behind one address share one store instead: anything with `first`."""

    def __init__(self, cap: int = SEEN_MAX):
        self._until: Dict[str, int] = {}
        self._cap = cap

    def first(self, hid: str, until: int, now: int) -> bool:
        if len(self._until) >= self._cap:
            for k in [k for k, u in self._until.items() if u <= now]:
                del self._until[k]
            while len(self._until) >= self._cap:
                del self._until[min(self._until, key=self._until.get)]
        if self._until.get(hid, 0) > now:
            return False
        self._until[hid] = until
        return True


class HandoffDoor:
    """The receiving side: admits each good handoff once.

    `keys` pins the game's key set ({key id: base64 public key}); with
    `client` instead, the door reads it from Elo Pros
    (`/api/store/serverkeys/{game}`), again every KEYS_REFRESH_S so a revoked
    key stops working, and at once (at most every KEYS_RETRY_S) when a handoff
    names a key it has not seen, so a rotated key works on its first use.
    """

    def __init__(self, game: str, names: Iterable[str], *, keys: Optional[Dict[str, str]] = None,
                 client=None, seen=None, clock: Callable[[], float] = time.time):
        if keys is None and client is None:
            raise ValueError("a door needs the game's keys, or a Client to read them with")
        self.game = game
        self.names = [str(n).lower() for n in names]
        self._pinned = keys is not None
        self._keys: Dict[str, bytes] = {k: base64.b64decode(v) for k, v in (keys or {}).items()}
        self._client = client
        self._seen = seen if seen is not None else MemorySeen()
        self._clock = clock
        self._read_at = 0.0
        self._tried_at = -1e18
        self._lock = threading.Lock()

    def _refresh(self, now: float) -> bool:
        self._tried_at = now
        got = self._client.server_keys(self.game)
        if not getattr(got, "ok", False):
            return False
        rows = (got.value or {}).get("keys") or []
        fresh = {}
        for r in rows:
            try:
                pub = base64.b64decode(str(r.get("pub")), validate=True)
            except (ValueError, TypeError):
                continue
            if len(pub) == 32 and isinstance(r.get("id"), str):
                fresh[r["id"]] = pub
        self._keys, self._read_at = fresh, now
        return True

    def keys(self) -> Dict[str, bytes]:
        return dict(self._keys)

    def admit(self, token: str, carry: bytes = b"") -> Handoff:
        now = self._clock()
        with self._lock:
            if not self._pinned:
                stale = now - self._read_at >= KEYS_REFRESH_S
                h = read_handoff(token)
                unknown = h.key not in self._keys and now - self._tried_at >= KEYS_RETRY_S
                if (stale or unknown) and not self._refresh(now) and not self._keys:
                    raise HandoffRefused("no_keys", "could not read the game's server keys; "
                                         "let the player join the usual way")
            got = verify_handoff(token, game=self.game, names=self.names, keys=self._keys,
                                 now=int(now), carry=carry)
            if not self._seen.first(got.id, got.exp + SKEW_S, int(now)):
                raise HandoffRefused("used", "this handoff was already admitted")
            return got


__all__ = ["CARRY_MAX", "CODES", "Handoff", "HandoffDoor", "HandoffRefused", "MemorySeen", "SKEW_S",
           "TTL_DEFAULT_S", "TTL_MAX_S", "handoff_text", "issue", "new_handoff_id", "read_handoff",
           "verify_handoff"]
