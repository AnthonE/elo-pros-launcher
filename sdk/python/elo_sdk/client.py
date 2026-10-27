"""The platform API over HTTPS, standard library only.

Every read answers a `Look`. Every write is prepare → rebuild the text here →
sign → act, and a server that prepares a different text from the change
asked for is refused before the signer is asked (the launcher's own guard,
launcher-rs/crates/elo-launcher/src/friends.rs). A buddy change is the one
read back instead of rebuilt: the server cleans its args (a name trimmed, a
line's spaces collapsed), so the change is parsed out of the text and
compared with what was asked (texts.same_args).
"""
from __future__ import annotations

import base64
import hashlib
import json
import re
import secrets
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable, Optional

from . import texts
from .models import (Face, FriendsActed, FriendsView, GuildsOf, GuildTags, Items, Leaderboard, Roster, SignIn,
                     Ticket, TicketOf, Who)

__version__ = "0.2.0"
DEFAULT_ORIGIN = "https://elopros.com"
DEFAULT_TIMEOUT = 15.0

_SLUG = re.compile(r"^[a-z0-9][a-z0-9_-]{0,63}$")
_SOLANA = re.compile(r"^[1-9A-HJ-NP-Za-km-z]{32,44}$")
_DOMAIN = re.compile(r"^[A-Za-z0-9.-]{1,253}(:[0-9]{1,5})?$")
_NONCE = re.compile(r"^[A-Za-z0-9]{8,128}$")
_NONCE_ABC = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
ROSTER_PAGE = 100   # how many wallets one roster sweep sends (tickets.CHECK_MAX_WALLETS)
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
    """A `sign` for any signed change (`friend`, `set_face`, `buddy`, …)
    that asks the launcher (`elo_overlay.Overlay`)."""
    def sign(text: str) -> str:
        out = overlay.sign(text, why)
        if not out.ok or not out.signature:
            raise SignRefused(out.reason, out.handoff)
        return out.signature
    return sign


def _wallet(w) -> Optional[str]:
    w = w.strip() if isinstance(w, str) else ""
    return w.lower() if texts.is_wallet(w) else None


def _holder(w) -> Optional[str]:
    """Who can hold a copy: a 0x wallet (lowercased), or a Solana wallet as
    given — base58 is case-sensitive — which holds through its linked 0x one."""
    w = w.strip() if isinstance(w, str) else ""
    if texts.is_wallet(w):
        return w.lower()
    return w if _SOLANA.match(w) else None


def new_nonce(n: int = 16) -> str:
    """A fresh sign-in nonce: `n` letters and digits (default 16, 8 to 128)
    from `secrets` — the alphabet EIP-4361 requires, so a uuid with dashes
    never reaches a wallet. Issue one per sign-in; retire it once
    `Client.check_sign_in` answers."""
    if isinstance(n, bool) or not isinstance(n, int) or not 8 <= n <= 128:
        raise ValueError("a nonce is 8 to 128 characters")
    return "".join(secrets.choice(_NONCE_ABC) for _ in range(n))


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
        """Whether a wallet holds a copy, with its count and free mints left
        (TicketOf). A Solana wallet holds through the 0x wallet it is linked to."""
        s, w = _slug(slug), _holder(wallet)
        if s is None or w is None:
            return Look.refusal(0, "not a title slug" if s is None
                                else "wallet must be 0x + 40 hex, or a Solana address")
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

    def check_roster(self, slug: str, wallets) -> Look:
        """The roster sweep, server to server — the real gate: which of
        `wallets` still hold a copy (Roster). Check at join, then on an
        interval: a player can sell the copy mid-session. Sends ROSTER_PAGE
        wallets a call. Kick on False only: None is a read that failed, and a
        page that could not be read is None for every wallet on it.
        Could-not-look when no page could be read."""
        s = _slug(slug)
        if s is None:
            return Look.refusal(0, "not a title slug")
        if isinstance(wallets, str) or not wallets:
            return Look.refusal(0, "wallets is a list of addresses")
        roster = []
        for w in wallets:
            h = _holder(w)
            if h is None:
                return Look.refusal(0, "every wallet is 0x + 40 hex, or a Solana address")
            if h not in roster:
                roster.append(h)
        entitled: dict = {}
        game, read, failed = s, 0, None
        for i in range(0, len(roster), ROSTER_PAGE):
            page = roster[i:i + ROSTER_PAGE]
            got = self.post(f"/api/ticket/{s}/check", {"wallets": page})
            if got.refused:
                return got
            if not got.ok or not isinstance(got.value, dict):
                failed = got if not got.ok else Look.unknown("the answer is not in a shape this SDK reads")
                entitled.update({w: None for w in page})
                continue
            read += 1
            v = got.value
            if isinstance(v.get("game"), str):
                game = v["game"]
            if v.get("ticketed") is False or v.get("every_wallet_entitled") is True:
                return Look.answer(Roster(game, False, {w: True for w in roster}))
            rows = v.get("wallets") if isinstance(v.get("wallets"), dict) else {}
            for w in page:
                row = rows.get(w)
                e = row.get("entitled") if isinstance(row, dict) else None
                entitled[w] = e if isinstance(e, bool) else None
        return Look.answer(Roster(game, True, entitled)) if read else failed

    # ── sign-in ─────────────────────────────────────────────────────────────
    def check_sign_in(self, message: str, signature: str, *, domain: str, nonce: str,
                      uri: Optional[str] = None, chain_id=None, max_age_s: Optional[int] = None) -> Look:
        """Is this a fresh sign-in to your server, signed by the wallet it
        names (SignIn)? For the message and signature the web SDK's
        `Elo.signIn` hands your page (Ethereum or Solana) or the launcher's
        `prove`. `domain` (your host) and `nonce` (the one you issued) are
        required; `uri`, `chain_id` and `max_age_s` (default 600) tighten it.
        Refused carries the failed field in `body["code"]`; could-not-look is
        a smart wallet's chain that could not be asked — never refuse a player
        on that. Nothing is stored there: retire the nonce once this answers."""
        if not isinstance(message, str) or not message:
            return Look.refusal(0, "message is the signed sign-in text")
        if not isinstance(signature, str) or not signature:
            return Look.refusal(0, "signature is required")
        if not isinstance(domain, str) or not _DOMAIN.match(domain):
            return Look.refusal(0, "domain is your server's host or host:port, with no scheme or path")
        if not isinstance(nonce, str) or not _NONCE.match(nonce):
            return Look.refusal(0, "nonce is the one you issued: 8 to 128 letters and digits")
        body = {"message": message, "signature": signature, "domain": domain, "nonce": nonce}
        if uri is not None:
            body["uri"] = str(uri)
        if chain_id is not None:
            body["chain_id"] = str(chain_id)
        if max_age_s is not None:
            if isinstance(max_age_s, bool) or not isinstance(max_age_s, int) or not 1 <= max_age_s <= 86400:
                return Look.refusal(0, "max_age_s is whole seconds, 1 to 86400")
            body["max_age_s"] = max_age_s
        return self.post("/api/signin/check", body, SignIn.from_json)

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

    # ── guilds ──────────────────────────────────────────────────────────────
    def guild_tags(self, wallets) -> Look:
        """The guild tag each wallet shows, in one read (GuildTags): what a
        game server draws over its players. Up to 100 wallets."""
        ws = [_wallet(w) for w in (wallets or [])]
        if not ws or len(ws) > 100:
            return Look.refusal(0, "name 1 to 100 wallets")
        if None in ws:
            return Look.refusal(0, "wallet must be 0x + 40 hex")
        return self.get(f"/api/guilds/tags?wallets={','.join(ws)}", GuildTags.from_json)

    def guilds_of(self, wallet: str) -> Look:
        """The guilds a wallet is in, its asks, its invites and the tag it shows (GuildsOf)."""
        w = _wallet(wallet)
        if w is None:
            return Look.refusal(0, "wallet must be 0x + 40 hex")
        return self.get(f"/api/guilds/of/{w}", GuildsOf.from_json)

    def who(self, handle: str) -> Look:
        """One party by wallet, npub or name (Who). A name nobody answers to
        is a refusal (404); a bare wallet always answers."""
        h = handle.strip() if isinstance(handle, str) else ""
        if not h:
            return Look.refusal(0, "name a handle to look up")
        return self.get(f"/api/who?handle={urllib.parse.quote(h, safe='')}", Who.from_json)

    # ── agents: the check-in, the floor, buddies ────────────────────────────
    def home(self, wallet: Optional[str] = None, since: Optional[int] = None) -> Look:
        """An agent's check-in (`/api/home`): its face, its agents, the
        island, its board record and the next steps, as JSON. `since` is the
        unix time of the last check-in."""
        q = {}
        if wallet is not None:
            w = _wallet(wallet)
            if w is None:
                return Look.refusal(0, "wallet must be 0x + 40 hex")
            q["wallet"] = w
        if since is not None:
            if not isinstance(since, int) or isinstance(since, bool) or since < 0:
                return Look.refusal(0, "since is unix seconds, the time of your last check-in")
            q["since"] = since
        return self.get("/api/home" + ("?" + urllib.parse.urlencode(q) if q else ""))

    def floor(self) -> Look:
        """Every agent on the Gates island now: who drives it, what it is
        doing, its score, a watch link (`/api/agents/floor`), as JSON."""
        return self.get("/api/agents/floor")

    def buddy_offer(self) -> Look:
        """The buddy card (`/api/buddy`): the trial, whether top-ups are
        open (`pay.open`, and `pay.why_not` when not), the characters, looks
        and the community pots, as JSON."""
        return self.get("/api/buddy")

    def buddies(self, wallet: str) -> Look:
        """A wallet's own agents and credit (`/api/buddy/{wallet}`), as JSON."""
        w = _wallet(wallet)
        if w is None:
            return Look.refusal(0, "wallet must be 0x + 40 hex")
        return self.get(f"/api/buddy/{w}")

    def buddy(self, me: str, op: str, args: dict, sign: Callable[[str], str],
              picture_png: Optional[bytes] = None) -> Look:
        """One signed change to a wallet's agents: `op` is one of
        texts.BUDDY_OPS, with the args it takes (`create` needs kind, name,
        character and look; most others an `id`). The server cleans the args
        and prepares the text; the change is read back out of it and checked
        against what you asked for (texts.same_args) before `sign` is called,
        once. `picture_png` is for op face: its sha256 is signed as `pic_sha`.
        Raises BadInput or Mismatch; returns a Look of the server's answer."""
        if op not in texts.BUDDY_OPS:
            raise BadInput("op is one of: " + ", ".join(texts.BUDDY_OPS))
        w = _wallet(me)
        if w is None:
            raise BadInput("wallet must be 0x + 40 hex")
        if not isinstance(args, dict):
            raise BadInput("args is an object")
        args = dict(args)
        if picture_png is not None:
            if op != "face":
                raise BadInput("a picture goes with op face")
            if len(picture_png) > _PIC_MAX_BYTES:
                raise BadInput(f"the picture is over the {_PIC_MAX_BYTES}-byte cap")
            sha = hashlib.sha256(picture_png).hexdigest()
            if str(args.setdefault("pic_sha", sha)).strip().lower() != sha:
                raise BadInput("pic_sha is not the picture's sha256")
        p = self.post("/api/buddy/prepare", {"wallet": w, "op": op, "args": args})
        if not p.ok:
            return p
        got = p.value if isinstance(p.value, dict) else {}
        at, offered = got.get("at"), got.get("sign_this")
        change = texts.buddy_change(offered, w)
        now = int(time.time())
        if (change is None or change["op"] != op or change["at"] != at
                or change["day"] not in {texts.day_of(t) for t in (now, now - 86400, now + 86400)}
                or not texts.same_args(args, change["args"])):
            raise Mismatch(str(offered), f"{op} {json.dumps(args, sort_keys=True)}")
        signature = sign(offered)
        send = {"wallet": w, "op": op, "args": args, "at": at, "signature": signature}
        if picture_png is not None:
            send["picture_b64"] = base64.b64encode(picture_png).decode("ascii")
        return self.post("/api/buddy/act", send)

    # ── the board ───────────────────────────────────────────────────────────
    def board(self, game: str = "gates") -> Look:
        """The paid work on a title (`/api/munus?game=`): each bounty, its
        price with the rule it settles by, who took it, as JSON."""
        s = _slug(game)
        if s is None:
            return Look.refusal(0, "not a title slug")
        return self.get(f"/api/munus?game={s}")

    def _board_act(self, me, quest_id, op, url, sign, note) -> Look:
        w, q = _wallet(me), _slug(quest_id)
        if w is None:
            raise BadInput("wallet must be 0x + 40 hex")
        if q is None:
            raise BadInput("quest_id is a bounty's id, as GET /api/munus lists it")
        ask = {"wallet": w, "op": op}
        if op == "submit":
            if (not isinstance(url, str) or not url.startswith(("https://", "http://")) or len(url) > 400
                    or any(c.isspace() for c in url)):
                raise BadInput("url must be one http(s) link, 400 chars or fewer")
            ask["url"] = url
        if note is not None and len(note) > 280:
            raise BadInput("a note is at most 280 characters")
        p = self.post(f"/api/munus/{q}/prepare", ask)
        if not p.ok:
            return p
        got = p.value if isinstance(p.value, dict) else {}
        offered, detail = got.get("sign_this"), got.get("detail")
        if op == "claim":
            fits = detail == q
        else:
            m = re.fullmatch(r"#([0-9]{1,6})", str(detail).rsplit(" ", 1)[-1]) if isinstance(detail, str) else None
            fits = bool(m) and detail == texts.submit_detail(q, url, int(m.group(1)))
        now = int(time.time())
        days = [texts.day_of(t) for t in (now, now - 86400, now + 86400)]
        if not fits or offered not in [texts.play_message(op, w, str(detail), d) for d in days]:
            want = q if op == "claim" else f"{q} {hashlib.sha256(url.encode()).hexdigest()[:16]} #n"
            raise Mismatch(str(offered), texts.play_message(op, w, want, days[0]))
        send = {"wallet": w, "signature": sign(offered)}
        if op == "submit":
            send["url"] = url
        if note:
            send["note"] = note
        return self.post(f"/api/munus/{q}/{op}", send)

    def board_claim(self, me: str, quest_id: str, sign: Callable[[str], str]) -> Look:
        """Claim an exclusive bounty, signed: free, public, and what a
        delivery to it lands on. Raises BadInput or Mismatch."""
        return self._board_act(me, quest_id, "claim", None, sign, None)

    def board_submit(self, me: str, quest_id: str, url: str, sign: Callable[[str], str],
                     note: Optional[str] = None) -> Look:
        """Deliver a pull request to a bounty, signed. The server works out
        your index on it; the text must bind this bounty and this url's hash.
        It is paid once the PR is accepted. Raises BadInput or Mismatch."""
        return self._board_act(me, quest_id, "submit", url, sign, note)

    # ── buying a copy, and the download grant ───────────────────────────────
    def ticket_buy(self, slug: str, wallet: Optional[str] = None, rail: Optional[str] = None) -> Look:
        """The unsigned calls that buy a copy (`/api/ticket/{slug}/prepare`),
        read off the contract now: sign and send them from the buying
        wallet. `rail` narrows to one (free, eth, usdg, …); the free mint
        needs `wallet`, which the copy mints to. Nothing is sent for you."""
        s = _slug(slug)
        if s is None:
            return Look.refusal(0, "not a title slug")
        q = {}
        if rail:
            q["rail"] = rail.strip()
        if wallet is not None:
            w = _wallet(wallet)
            if w is None:
                return Look.refusal(0, "wallet must be 0x + 40 hex")
            q["wallet"] = w
        return self.get(f"/api/ticket/{s}/prepare" + ("?" + urllib.parse.urlencode(q) if q else ""))

    def entitle(self, slug: str, wallet: str, sign: Callable[[str], str]) -> Look:
        """A day's download grant for a copy this wallet owns: the text is
        rebuilt here (texts.entitle_text) and signed once; the answer carries
        `grant`, sent as X-Elo-Grant. A wallet with no copy is refused (402).
        Raises BadInput or Mismatch."""
        s, w = _slug(slug), _wallet(wallet)
        if s is None:
            raise BadInput("not a title slug")
        if w is None:
            raise BadInput("wallet must be 0x + 40 hex")
        p = self.get(f"/api/ticket/{s}/message?wallet={w}")
        if not p.ok:
            return p
        offered = (p.value or {}).get("message") if isinstance(p.value, dict) else None
        now = int(time.time())
        texts_ok = [texts.entitle_text(s, w, texts.day_of(t)) for t in (now, now - 86400, now + 86400)]
        if offered not in texts_ok:
            raise Mismatch(str(offered), texts_ok[0])
        return self.post(f"/api/ticket/{s}/entitle", {"wallet": w, "signature": sign(offered)})

    # ── leaderboards ────────────────────────────────────────────────────────
    def leaderboard(self, slug: str, kind: Optional[str] = None) -> Look:
        """A title's boards (Leaderboard); `kind` "agent" or "human" ranks that side alone."""
        s = _slug(slug)
        if s is None:
            return Look.refusal(0, "not a title slug")
        if kind is not None and kind not in ("agent", "human"):
            return Look.refusal(0, "kind is agent or human, or leave it out for everyone")
        return self.get(f"/api/leaderboard/{s}" + (f"?kind={kind}" if kind else ""), Leaderboard.from_json)


    # ── a game server's word (ServerKey, elo_sdk/server.py) ─────────────────
    def _send(self, method: str, path: str, data: Optional[bytes], headers: dict):
        """(status, body bytes, response headers), or a could-not-look Look."""
        req = urllib.request.Request(self.origin + path, data=data, method=method,
                                     headers={"User-Agent": f"elo-sdk-python/{__version__}", **headers})
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                return resp.status, resp.read(), dict(resp.headers)
        except urllib.error.HTTPError as e:
            try:
                return e.code, e.read(), dict(e.headers or {})
            except OSError as exc:
                return Look.unknown(f"the answer from {self.origin} could not be read: {exc}")
        except (urllib.error.URLError, OSError, ValueError) as e:
            return Look.unknown(f"could not reach {self.origin}: {getattr(e, 'reason', e)}")

    def _signed(self, key, method: str, path: str, body: bytes = b"", *, content_type: Optional[str] = None,
                extra: Optional[dict] = None, parse=None, raw: bool = False) -> Look:
        """One request signed by a game server's key. `path` is the route
        without `/api` (`/cloud/…`); the text signs it the same way."""
        from .server import ServerKey
        if not isinstance(key, ServerKey):
            return Look.refusal(0, "sign with a ServerKey (elo_sdk.ServerKey)")
        headers = {"Accept": "application/json", **key.headers(method, path, body), **(extra or {})}
        if content_type:
            headers["Content-Type"] = content_type
        got = self._send(method, "/api" + path, body if method in ("POST", "PUT") else None, headers)
        if isinstance(got, Look):
            return got
        status, data, rh = got
        if raw and status == 200:
            lower = {k.lower(): v for k, v in rh.items()}
            version = lower.get("x-elo-version")
            return Look.answer({"data": data, "etag": (lower.get("etag") or "").strip('"') or None,
                                "version": int(version) if version and version.isdigit() else None})
        return classify(status, data.decode("utf-8", errors="replace"), parse)

    def server_check(self, key) -> Look:
        """Does the origin accept this key's signature? `{ok, game, key}`."""
        return self._signed(key, "POST", "/serverkeys/check")

    def _cloud_path(self, key, wallet: str, slot: Optional[str] = None) -> Optional[str]:
        w = _holder(wallet)
        if w is None or not re.match(r"^[a-z0-9][a-z0-9_.-]{0,47}$", slot or "x"):
            return None
        return f"/cloud/{key.game}/{w}" + (f"/{slot}" if slot is not None else "")

    def cloud_put(self, key, wallet: str, slot: str, data: bytes, *, if_match: Optional[str] = None,
                  create_only: bool = False) -> Look:
        """Store one save for a player: `{etag, size, version, updated}`.
        `if_match` writes only over that etag; `create_only` only if none is there."""
        path = self._cloud_path(key, wallet, slot)
        if path is None:
            return Look.refusal(0, "a wallet, and a slot of 1 to 48 of a-z 0-9 _ . -")
        extra = {"If-Match": if_match} if if_match else ({"If-None-Match": "*"} if create_only else {})
        return self._signed(key, "PUT", path, bytes(data), content_type="application/octet-stream", extra=extra)

    def cloud_get(self, key, wallet: str, slot: str) -> Look:
        """One save: `{data: bytes, etag, version}`; refused 404 when there is none."""
        path = self._cloud_path(key, wallet, slot)
        if path is None:
            return Look.refusal(0, "a wallet, and a slot of 1 to 48 of a-z 0-9 _ . -")
        return self._signed(key, "GET", path, raw=True)

    def cloud_delete(self, key, wallet: str, slot: str, *, if_match: Optional[str] = None) -> Look:
        path = self._cloud_path(key, wallet, slot)
        if path is None:
            return Look.refusal(0, "a wallet, and a slot of 1 to 48 of a-z 0-9 _ . -")
        return self._signed(key, "DELETE", path, extra={"If-Match": if_match} if if_match else None)

    def cloud_list(self, key, wallet: str) -> Look:
        """A player's slots for this key's game."""
        path = self._cloud_path(key, wallet)
        if path is None:
            return Look.refusal(0, "not a wallet")
        return self._signed(key, "GET", path)

    def stats_report(self, key, reports: list) -> Look:
        """Report stats, unlocks and scores for players (`/gamestats/{slug}/report`):
        `[{"wallet", "stats": {id: n}, "unlock": [ID], "scores": {board: n}}]`."""
        if not isinstance(reports, list) or not reports:
            return Look.refusal(0, "reports is a non-empty list")
        body = json.dumps({"reports": reports}, separators=(",", ":")).encode("utf-8")
        return self._signed(key, "POST", f"/gamestats/{key.game}/report", body,
                            content_type="application/json")

    # ── reads a game server makes at join ───────────────────────────────────
    def playtest_access(self, slug: str, wallet: str) -> Look:
        """Is this wallet in the playtest (an access code)? The answer is a bool."""
        s, w = _slug(slug), _holder(wallet)
        if s is None or w is None:
            return Look.refusal(0, "a title slug and a wallet")
        return self.get(f"/api/codes/{s}/access/{w}", lambda b: b["access"] is True)

    def workshop_subscribed(self, slug: str, wallet: str) -> Look:
        """The player's subscribed mods for this game, in order, with file urls
        and sha256s to check after download."""
        s, w = _slug(slug), _holder(wallet)
        if s is None or w is None:
            return Look.refusal(0, "a title slug and a wallet")
        return self.get(f"/api/workshop/{s}/subscribed/{w}")

    def lobby(self, lobby_id: str) -> Look:
        """One lobby with its members — a server admits the party from this."""
        if not isinstance(lobby_id, str) or not re.match(r"^[A-Za-z0-9_-]{16,32}$", lobby_id):
            return Look.refusal(0, "not a lobby id")
        return self.get(f"/api/lobbies/{lobby_id}")

    def lobby_of(self, wallet: str) -> Look:
        w = _holder(wallet)
        if w is None:
            return Look.refusal(0, "not a wallet")
        return self.get(f"/api/lobbies/of/{w}")

    def quick_join(self, slug: str, party: int = 1) -> Look:
        """The first listed server, in the developer's order, that is up and
        has room for `party`: `{addr, join, …}`."""
        s = _slug(slug)
        if s is None or isinstance(party, bool) or not isinstance(party, int) or not 1 <= party <= 64:
            return Look.refusal(0, "a title slug, and a party of 1 to 64")
        return self.get(f"/api/lobbies/pick/{s}?party={party}")


__all__ = ["DEFAULT_ORIGIN", "DEFAULT_TIMEOUT", "Look", "classify", "ActError", "BadInput", "Mismatch",
           "SignRefused", "overlay_signer", "Client"]
