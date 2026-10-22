"""The platform API over HTTPS, standard library only.

Every read answers a `Look`. Every write is prepare → rebuild the text here →
sign → act, and a server that prepares a different text from the change
asked for is refused before the signer is asked (the launcher's own guard,
launcher-rs/crates/elo-launcher/src/friends.rs).
"""
from __future__ import annotations

import base64
import hashlib
import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable, Optional

from . import texts
from .models import Face, FriendsActed, FriendsView, Items, Leaderboard, Ticket, TicketOf, Who

__version__ = "0.1.0"
DEFAULT_ORIGIN = "https://elopros.com"
DEFAULT_TIMEOUT = 15.0

_SLUG = re.compile(r"^[a-z0-9][a-z0-9_-]{0,63}$")
_NAME_MAX, _LABEL_MAX, _PIC_MAX_BYTES = 16, 48, 256 * 1024
_LABEL_CHARS = frozenset("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 ._:/@+-")


class Look:
    """One call's outcome: `kind` is "answer", "refused" or "could_not_look".

    - answer: the server answered; the answer is `value`.
    - refused: the server said no (`status`, `error`, the whole `body`), or
      this SDK did before asking (`status` 0).
    - could_not_look: unreachable, timed out, a 5xx, a body that is not JSON
      or not the expected shape, or the server saying it could not read
      (`reachable: false`). `error` says why. Ask again later; never read it
      as "no" or "nothing".
    """
    __slots__ = ("kind", "value", "status", "error", "body")

    def __init__(self, kind: str, value: Any = None, status: Optional[int] = None,
                 error: Optional[str] = None, body: Any = None):
        self.kind, self.value, self.status, self.error, self.body = kind, value, status, error, body

    @classmethod
    def answer(cls, value) -> "Look":
        return cls("answer", value=value)

    @classmethod
    def refusal(cls, status: int, error: str, body: Any = None) -> "Look":
        return cls("refused", status=status, error=error, body=body)

    @classmethod
    def unknown(cls, why: str) -> "Look":
        return cls("could_not_look", error=why)

    @property
    def ok(self) -> bool:
        return self.kind == "answer"

    @property
    def refused(self) -> bool:
        return self.kind == "refused"

    @property
    def could_not_look(self) -> bool:
        return self.kind == "could_not_look"

    def map(self, fn: Callable[[Any], Any]) -> "Look":
        return Look.answer(fn(self.value)) if self.ok else self

    def __eq__(self, other) -> bool:
        return isinstance(other, Look) and all(getattr(self, k) == getattr(other, k) for k in self.__slots__)

    def __repr__(self) -> str:
        if self.ok:
            return f"Look.answer({self.value!r})"
        if self.refused:
            return f"Look.refusal({self.status}, {self.error!r})"
        return f"Look.unknown({self.error!r})"


def _said(body, keys) -> Optional[str]:
    if isinstance(body, dict):
        for k in keys:
            if isinstance(body.get(k), str):
                return body[k]
    return None


def classify(status: int, text: str, parse: Optional[Callable[[Any], Any]] = None) -> Look:
    """Sort one HTTP response into a Look — the rule every call shares, and
    the same rule the Rust and JavaScript SDKs apply:

    1. a body that is not JSON → could not look;
    2. a 5xx → could not look (the server's `why` or `error` when it gave one);
    3. any other non-2xx → refused, with the server's `error` (or `detail`);
    4. a 2xx whose body says `reachable: false` → could not look;
    5. a 2xx that `parse` cannot read → could not look;
    6. otherwise the answer, through `parse` when given.
    """
    try:
        body = json.loads(text)
    except (TypeError, ValueError):
        return Look.unknown(f"the server answered {status} with a body that is not JSON")
    if status >= 500:
        return Look.unknown(_said(body, ("why", "error")) or f"the server answered {status}")
    if not 200 <= status < 300:
        return Look.refusal(status, _said(body, ("error", "why", "detail")) or f"the server answered {status}",
                           body)
    if isinstance(body, dict) and body.get("reachable") is False:
        return Look.unknown(_said(body, ("why", "error")) or "the server could not look")
    if parse is None:
        return Look.answer(body)
    if not isinstance(body, dict):
        return Look.unknown("the answer is not in a shape this SDK reads")
    try:
        return Look.answer(parse(body))
    except Exception as e:  # noqa: BLE001 — a shape we cannot read is not an answer
        return Look.unknown(f"the answer is not in a shape this SDK reads: {e}")


class ActError(Exception):
    """A signed change that was not sent. A refusal from the server, or a
    server that could not be reached, is the returned Look instead."""


class BadInput(ActError, ValueError):
    """The change is not one the server takes; nothing was sent."""


class Mismatch(ActError):
    """The server prepared a different text from the change asked for.
    Nothing was signed."""

    def __init__(self, offered: str, expected: str):
        super().__init__("the origin prepared a different change from the one asked for — nothing was signed")
        self.offered, self.expected = offered, expected


class SignRefused(ActError):
    """The launcher did not sign (`overlay_signer`). `handoff` is a url when
    the player finishes in a browser instead — show it; do not retry."""

    def __init__(self, reason: str, handoff: Optional[str] = None):
        super().__init__(reason or (f"finish this in your browser: {handoff}" if handoff else "not signed"))
        self.handoff = handoff


def overlay_signer(overlay, why: str = "") -> Callable[[str], str]:
    """A `sign` for `friend`/`set_face` that asks the launcher (`elo_overlay.Overlay`)."""
    def sign(text: str) -> str:
        out = overlay.sign(text, why)
        if not out.ok or not out.signature:
            raise SignRefused(out.reason, out.handoff)
        return out.signature
    return sign


def _wallet(w) -> Optional[str]:
    w = w.strip() if isinstance(w, str) else ""
    return w.lower() if texts.is_wallet(w) else None


def _slug(s) -> Optional[str]:
    s = s.strip().lower() if isinstance(s, str) else ""
    return s if _SLUG.match(s) else None


def _name_error(name: str) -> Optional[str]:
    if len(name) > _NAME_MAX:
        return f"a name is at most {_NAME_MAX} characters"
    if any(not (0x20 <= ord(c) <= 0x7E) for c in name):
        return "a name uses plain letters, digits, spaces and punctuation only"
    if name[:2].lower() == "0x" or name.startswith("#"):
        return "a name may not start with 0x or #"
    return None


def _label_error(field: str, v: Optional[str]) -> Optional[str]:
    if not v:
        return None
    if len(v) > _LABEL_MAX or not set(v) <= _LABEL_CHARS:
        return (f"{field} is 1 to {_LABEL_MAX} characters: letters, digits, spaces and "
                '. _ : / @ + - (or "" to clear it)')
    return None


class Client:
    """A client for one origin (https://elopros.com, or a staging origin)."""

    def __init__(self, origin: str = DEFAULT_ORIGIN, timeout: float = DEFAULT_TIMEOUT):
        self.origin = origin.rstrip("/")
        self.timeout = timeout

    # ── transport ───────────────────────────────────────────────────────────
    def _request(self, method: str, path: str, body: Optional[dict], parse) -> Look:
        data = None if body is None else json.dumps(body).encode("utf-8")
        headers = {"Accept": "application/json", "User-Agent": f"elo-sdk-python/{__version__}"}
        if data is not None:
            headers["Content-Type"] = "application/json"
        req = urllib.request.Request(self.origin + path, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                status, raw = resp.status, resp.read()
        except urllib.error.HTTPError as e:
            status = e.code
            try:
                raw = e.read()
            except OSError as exc:
                return Look.unknown(f"the answer from {self.origin} could not be read: {exc}")
        except (urllib.error.URLError, OSError, ValueError) as e:
            return Look.unknown(f"could not reach {self.origin}: {getattr(e, 'reason', e)}")
        return classify(status, raw.decode("utf-8", errors="replace"), parse)

    def get(self, path: str, parse=None) -> Look:
        """GET a path under the origin (`/api/…`) and classify the answer."""
        return self._request("GET", path, None, parse)

    def post(self, path: str, body: dict, parse=None) -> Look:
        """POST a JSON body to a path under the origin and classify the answer."""
        return self._request("POST", path, body, parse)

    # ── friends ─────────────────────────────────────────────────────────────
    def friends(self, wallet: str) -> Look:
        """A wallet's friends, buddies and asks, with who is playing what (FriendsView)."""
        w = _wallet(wallet)
        if w is None:
            return Look.refusal(0, "wallet must be 0x + 40 hex")
        return self.get(f"/api/friends/{w}", FriendsView.from_json)

    def friend(self, me: str, op: str, other: str, sign: Callable[[str], str]) -> Look:
        """One signed friends change: `op` is ask, accept, decline, cancel,
        remove, block or unblock, on the wallet `other`, signed as `me`.
        `sign(text)` returns the 0x… signature — `overlay_signer(overlay)`, a
        hardware wallet, or your own. It is called at most once, and never for
        a text this SDK did not rebuild from the change itself. Raises
        BadInput or Mismatch, and whatever `sign` raises; returns a Look of
        FriendsActed."""
        key = texts.op_key(op)
        if key is None:
            raise BadInput("op is one of: " + ", ".join(texts.OPS))
        w, o = _wallet(me), _wallet(other)
        if w is None:
            raise BadInput("wallet must be 0x + 40 hex")
        if o is None:
            raise BadInput(f"{key} is a wallet address: 0x and 40 hex characters")
        if w == o:
            raise BadInput("that's your own wallet")
        p = self.post("/api/friends/prepare", {"wallet": w, "op": op, "args": {key: o}})
        if not p.ok:
            return p
        got = p.value if isinstance(p.value, dict) else {}
        at, offered = got.get("at"), got.get("sign_this")
        if not isinstance(at, int) or isinstance(at, bool):
            raise Mismatch(str(offered), "")
        expected = texts.friends_sign_text(w, op, o, at)
        if offered != expected:
            raise Mismatch(str(offered), expected or "")
        signature = sign(expected)
        return self.post("/api/friends/act", {"wallet": w, "op": op, "args": {key: o}, "at": at,
                                              "signature": signature}, FriendsActed.from_json)

    # ── tickets ─────────────────────────────────────────────────────────────
    def ticket(self, slug: str) -> Look:
        """A title's ticket card (Ticket). Free titles answer `ticketed` False."""
        s = _slug(slug)
        if s is None:
            return Look.refusal(0, "not a title slug")
        return self.get(f"/api/ticket/{s}", Ticket.from_json)

    def ticket_of(self, slug: str, wallet: str) -> Look:
        """Whether a wallet holds a copy, with its count and free mints left (TicketOf)."""
        s, w = _slug(slug), _wallet(wallet)
        if s is None or w is None:
            return Look.refusal(0, "not a title slug" if s is None else "wallet must be 0x + 40 hex")
        return self.get(f"/api/ticket/{s}/of/{w}", TicketOf.from_json)

    def owns(self, slug: str, wallet: str) -> Look:
        """Does this wallet own a copy: an answer of True or False, or
        could-not-look. Kick a player on a definite False only."""
        got = self.ticket_of(slug, wallet)
        if not got.ok:
            return got
        if got.value.entitled is None:
            return Look.unknown(got.value.why or "the ownership read failed")
        return Look.answer(got.value.entitled)

    # ── items ───────────────────────────────────────────────────────────────
    def items(self, wallet: str, title: Optional[str] = None) -> Look:
        """What a wallet holds (Items); `title` keeps only that title's items."""
        w = _wallet(wallet)
        if w is None:
            return Look.refusal(0, "wallet must be 0x + 40 hex")
        q = f"?title={urllib.parse.quote(title.strip(), safe='')}" if title else ""
        return self.get(f"/api/items/of/{w}{q}", Items.from_json)

    # ── faces and profiles ──────────────────────────────────────────────────
    def face(self, wallet: str) -> Look:
        """A wallet's face: name, picture, and what it says it is (Face)."""
        w = _wallet(wallet)
        if w is None:
            return Look.refusal(0, "wallet must be 0x + 40 hex")
        return self.get(f"/api/face/{w}", Face.from_json)

    def set_face(self, wallet: str, sign: Callable[[str], str], *, name: Optional[str] = None,
                 picture_png: Optional[bytes] = None, clear_picture: bool = False,
                 kind: Optional[str] = None, harness: Optional[str] = None,
                 model: Optional[str] = None) -> Look:
        """Change a wallet's face, signed. None leaves a field as it is; ""
        clears it. The picture is a square 8-bit RGB/RGBA PNG, 64-512 px, at
        most 256 KiB. Raises BadInput or Mismatch; returns a Look of Face."""
        w = _wallet(wallet)
        if w is None:
            raise BadInput("wallet must be 0x + 40 hex")
        if name is not None:
            name = name.strip()
            bad = _name_error(name) if name else None
            if bad:
                raise BadInput(bad)
        if kind is not None and kind not in ("agent", "human"):
            raise BadInput('kind is "agent" or "human" (or leave it out to keep the current one)')
        bad = _label_error("harness", harness) or _label_error("model", model)
        if bad:
            raise BadInput(bad)
        if picture_png is not None and clear_picture:
            raise BadInput("send a picture or clear it, not both")
        if picture_png is not None and len(picture_png) > _PIC_MAX_BYTES:
            raise BadInput(f"the picture is over the {_PIC_MAX_BYTES}-byte cap")
        sha = hashlib.sha256(picture_png).hexdigest() if picture_png is not None else ("" if clear_picture else None)
        if name is None and sha is None and kind is None and harness is None and model is None:
            raise BadInput("nothing to change — send a name, a picture, a kind, a harness or a model")
        fields = {"name": name, "picture_sha256": sha, "kind": kind, "harness": harness, "model": model}
        p = self.post("/api/face/prepare", {"wallet": w, **{k: v for k, v in fields.items() if v is not None}})
        if not p.ok:
            return p
        got = p.value if isinstance(p.value, dict) else {}
        at, offered = got.get("at"), got.get("sign_this")
        if not isinstance(at, int) or isinstance(at, bool):
            raise Mismatch(str(offered), "")
        detail = texts.face_detail(at, name, sha, kind, harness, model)
        now = int(time.time())
        text = next((t for t in (texts.play_message(texts.FACE_ACTION, w, detail, texts.day_of(s))
                                 for s in (now, now - 86400, now + 86400))
                     if t == offered and got.get("detail") == detail), None)
        if text is None:
            raise Mismatch(str(offered), texts.play_message(texts.FACE_ACTION, w, detail, texts.day_of(now)))
        signature = sign(text)
        send = {"wallet": w, **{k: v for k, v in (("name", name), ("kind", kind), ("harness", harness),
                                                  ("model", model)) if v is not None},
                "clear_picture": bool(clear_picture), "at": at, "signature": signature}
        if picture_png is not None:
            send["picture_b64"] = base64.b64encode(picture_png).decode("ascii")
        return self.post("/api/face", send, Face.from_json)

    def who(self, handle: str) -> Look:
        """One party by sworn name, npub, vow id or wallet (Who). A handle
        nobody has sworn is a refusal (404); a bare wallet always answers."""
        h = handle.strip() if isinstance(handle, str) else ""
        if not h:
            return Look.refusal(0, "name a handle to look up")
        return self.get(f"/api/who?handle={urllib.parse.quote(h, safe='')}", Who.from_json)

    # ── leaderboards ────────────────────────────────────────────────────────
    def leaderboard(self, slug: str, kind: Optional[str] = None) -> Look:
        """A title's boards (Leaderboard); `kind` "agent" or "human" ranks that side alone."""
        s = _slug(slug)
        if s is None:
            return Look.refusal(0, "not a title slug")
        if kind is not None and kind not in ("agent", "human"):
            return Look.refusal(0, "kind is agent or human, or leave it out for everyone")
        return self.get(f"/api/leaderboard/{s}" + (f"?kind={kind}" if kind else ""), Leaderboard.from_json)


__all__ = ["DEFAULT_ORIGIN", "DEFAULT_TIMEOUT", "Look", "classify", "ActError", "BadInput", "Mismatch",
           "SignRefused", "overlay_signer", "Client"]
