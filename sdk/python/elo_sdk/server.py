"""A game server's key, and the calls that take a game server's word.

Your servers sign their writes with an Ed25519 key you named on the dev desk
(act `serverkeys`) or in `elo.json` (`server_keys`). We keep the public half
only; the private key stays on your server. This is Steam's Game Server Login
Token, without a token anyone could leak from us.

    from elo_sdk import Client, ServerKey

    key = ServerKey.from_file("my-game", "eu-1", "/etc/my-game/server.key")
    print(key.public_b64)            # name this on the desk, or in elo.json
    elo = Client()
    elo.server_check(key)            # a Look: does the origin accept our signature?
    elo.cloud_put(key, wallet, "save", b"...")

Make a key with OpenSSL (`openssl genpkey -algorithm ed25519 -out server.key`)
or `ServerKey.generate`. Signing needs the `cryptography` package, the
`server` extra:

    pip install "elo-sdk[server] @ git+https://github.com/AnthonE/elo-pros-launcher#subdirectory=sdk/python"

Everything else in this SDK is standard library only.

The signed text, which every SDK and the meter build the same way
(sdk/testdata/server_texts.json pins it):

    elo server
    <METHOD>
    <path without /api, with ?query>
    <slug>/<key id>
    <unix seconds>
    <nonce>
    <sha256 hex of the body>
"""
from __future__ import annotations

import base64
import hashlib
import re
import secrets
import time
from typing import Optional

_ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,30}$")
_SLUG = re.compile(r"^[a-z0-9][a-z0-9-]{0,62}$")
_NONCE_ABC = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"


def server_text(method: str, path: str, who: str, at: str, body: bytes = b"",
                nonce: Optional[str] = None) -> str:
    """The exact text a server signs for one request (`meter/serverkeys.py`)."""
    tail = f"{nonce}\n" if nonce else ""
    return (f"elo server\n{method.upper()}\n{path}\n{who}\n{at}\n{tail}"
            f"{hashlib.sha256(body or b'').hexdigest()}")


def new_server_nonce(n: int = 22) -> str:
    """A fresh request nonce: `n` (8 to 64) of letters, digits, '-' and '_'."""
    if isinstance(n, bool) or not isinstance(n, int) or not 8 <= n <= 64:
        raise ValueError("a request nonce is 8 to 64 characters")
    return "".join(secrets.choice(_NONCE_ABC) for _ in range(n))


def _ed25519():
    try:
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    except ImportError as e:  # pragma: no cover — the extra is not installed
        raise ImportError('signing as a game server needs the cryptography package — the '
                          '"server" extra of elo-sdk, or pip install cryptography') from e
    return Ed25519PrivateKey, serialization


class ServerKey:
    """One of a game's server keys: signs requests the way the origin checks them."""

    def __init__(self, game: str, key_id: str, seed: bytes):
        if not isinstance(game, str) or not _SLUG.match(game):
            raise ValueError("game is the title's slug")
        if not isinstance(key_id, str) or not _ID.match(key_id):
            raise ValueError("key_id is lowercase letters, digits and '-', up to 31 characters")
        if not isinstance(seed, (bytes, bytearray)) or len(seed) != 32:
            raise ValueError("an Ed25519 private key is a 32-byte seed")
        Ed25519PrivateKey, serialization = _ed25519()
        self.game, self.key_id = game, key_id
        self._key = Ed25519PrivateKey.from_private_bytes(bytes(seed))
        raw = self._key.public_key().public_bytes(serialization.Encoding.Raw,
                                                  serialization.PublicFormat.Raw)
        self.public_b64 = base64.b64encode(raw).decode("ascii")

    @property
    def who(self) -> str:
        return f"{self.game}/{self.key_id}"

    @classmethod
    def generate(cls, game: str, key_id: str) -> "ServerKey":
        """A new key. Keep `seed_hex()` on the server; name `public_b64` with us."""
        return cls(game, key_id, secrets.token_bytes(32))

    @classmethod
    def from_pem(cls, game: str, key_id: str, pem: bytes) -> "ServerKey":
        """From a PKCS#8 PEM, the file `openssl genpkey -algorithm ed25519` writes."""
        Ed25519PrivateKey, serialization = _ed25519()
        k = serialization.load_pem_private_key(pem, password=None)
        if not isinstance(k, Ed25519PrivateKey):
            raise ValueError("that PEM is not an Ed25519 private key")
        seed = k.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw,
                               serialization.NoEncryption())
        return cls(game, key_id, seed)

    @classmethod
    def from_file(cls, game: str, key_id: str, path: str) -> "ServerKey":
        """From a PEM file, or a file holding the 64-hex seed."""
        with open(path, "rb") as f:
            raw = f.read()
        text = raw.strip()
        if re.fullmatch(rb"[0-9a-fA-F]{64}", text):
            return cls(game, key_id, bytes.fromhex(text.decode()))
        return cls.from_pem(game, key_id, raw)

    def seed_hex(self) -> str:
        """The private seed, for your server's own disk. Never send it anywhere."""
        _, serialization = _ed25519()
        return self._key.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw,
                                       serialization.NoEncryption()).hex()

    def sign_text(self, text: str) -> str:
        return base64.b64encode(self._key.sign(text.encode("utf-8"))).decode("ascii")

    def headers(self, method: str, path: str, body: bytes = b"", *, at: Optional[int] = None,
                nonce: Optional[str] = None) -> dict:
        """The four headers for one request. `path` is what you request, with
        or without `/api`, with its `?query`."""
        at_s = str(int(time.time()) if at is None else int(at))
        nonce = new_server_nonce() if nonce is None else nonce
        text = server_text(method, path, self.who, at_s, body, nonce)
        return {"X-Elo-Server": self.who, "X-Elo-Time": at_s, "X-Elo-Nonce": nonce,
                "X-Elo-Signature": self.sign_text(text)}

    def handoff(self, player: str, frm: str, to: str, *, carry: Optional[bytes] = None,
                ttl: int = 60, now: Optional[int] = None, hid: Optional[str] = None) -> str:
        """A handoff moving `player` from this server (`frm`, host:port) to
        another of this game's servers (`to`). Give the client the token, the
        address and `carry` if any (docs/client/HANDOFF.md)."""
        from .handoff import issue
        return issue(self._key.sign, self.game, self.key_id, player, frm, to,
                     carry=carry, ttl=ttl, now=now, hid=hid)

    def __repr__(self) -> str:
        return f"ServerKey({self.who!r}, public={self.public_b64!r})"


__all__ = ["ServerKey", "server_text", "new_server_nonce"]
