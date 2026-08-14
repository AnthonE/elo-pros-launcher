"""scry overlay — the Python reference client. One file, stdlib only.

What a game or an agent uses to reach a running scry launcher: who is playing,
a signature, the catalog, the shard list. It holds no key and has no code path
that could — see `sdk/PROTOCOL.md` and `docs/client/SDK.md`.

    from scry_overlay import Overlay

    scry = Overlay(game="gates", version="0.1.0")
    if scry.connect():
        who = scry.address()                       # 0x… or None
        out = scry.sign(play_message("duel", vow_id, "ETH up 5"))
        if out.ok:
            post(signature=out.signature)
        elif out.handoff:
            show_link(out.handoff)                 # the player finishes in a browser
        else:
            show(out.reason)

**Every method works when there is no launcher.** `connect()` returns False,
`address()` returns None, `sign()` returns a `Result` explaining that nothing
is there. A game must play fine without a launcher, and the shape of this
class is what makes forgetting that hard.

Vendor it. It is one file with no dependencies, and a game pinning a copy is
better than a game tracking a package.
"""
from __future__ import annotations

import json
import os
import socket
from dataclasses import dataclass
from pathlib import Path

PROTOCOL = 1
SOCKET_ENV = "SCRY_LAUNCHER_SOCKET"
DEFAULT_TIMEOUT = 300.0        # a consent prompt waits for a person

FAMILIES = ("braid", "card", "covenant", "doc", "familiar", "hive", "holder",
            "meter", "pact", "play", "review", "store", "vow")


def default_socket() -> str:
    """Where the door is when nobody named one.

    ⚠ Must agree exactly with the launcher's own default
    (`scry-broker/src/transport.rs`) and with the Rust SDK's. A mismatch is the
    worst shape of bug here: a game finds no launcher on a machine that is
    running one, reports the normal "playing anonymously", and nothing anywhere
    goes red. `$SCRY_LAUNCHER_SOCKET` is set by the launcher on every native
    build it starts and wins over both defaults.
    """
    env = os.environ.get(SOCKET_ENV)
    if env:
        return env
    if os.name == "nt":
        # One pipe per user: the Windows pipe namespace is machine-wide, and
        # two people signed into one box must not land on the same door.
        user = "".join(c for c in os.environ.get("USERNAME", "")
                       if c.isalnum() or c in "-_") or "default"
        return rf"\\.\pipe\scry-launcher-{user}"
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if runtime:
        return str(Path(runtime) / "scry" / "launcher.sock")
    return str(Path.home() / ".cache" / "scry" / "launcher" / "launcher.sock")


class _PipeConn:
    """A Windows named pipe wearing just enough of a socket to stand in for one.

    CPython does not expose `socket.AF_UNIX` on Windows, so the door there is a
    named pipe — which opens as an ordinary file. Only three methods are used
    by the client below, so only three are provided: a fuller fake would invite
    someone to rely on a part that was never tested.

    ⚠ **The timeout is not enforced on reads here**, the same as the Rust SDK's
    Windows half and for the same reason: bounding them wants overlapped I/O.
    A launcher that accepts and then never answers hangs the caller instead of
    erroring. That is a launcher bug either way; call the SDK off your main
    thread, which a consent prompt deserves regardless.
    """

    __slots__ = ("_f",)

    def __init__(self, path: str, timeout: float):
        self._f = open(path, "r+b", buffering=0)

    def sendall(self, data: bytes) -> None:
        self._f.write(data)

    def recv(self, n: int) -> bytes:
        return self._f.read(n) or b""

    def close(self) -> None:
        self._f.close()


@dataclass
class Result:
    """One reply. `ok` is the only field always meaningful.

    `handoff` is NOT an error — it means the player's signer is their browser
    and the act finishes there. A game that retries on it will spin.
    """
    ok: bool
    signature: str | None = None
    address: str | None = None
    family: str = ""
    backend: str = ""
    reason: str = ""
    handoff: str | None = None
    refused_by_player: bool = False
    raw: dict | None = None

    @property
    def needs_browser(self) -> bool:
        return bool(self.handoff)


@dataclass
class Profile:
    """What the town says about one address. Every field is a claim about a
    stranger, and `sworn` is the only thing separating a checked name from a
    typed one.

    `found=False` is a real answer — *we looked, there is no sworn identity*.
    It is not the same as `Overlay.profile()` returning `None`, which means we
    could not look at all.
    """
    raw: dict
    reachable: bool = True
    found: bool = True

    @property
    def handle(self) -> str | None:
        """The SWORN name — in the locked index, and it cost SCRY to change."""
        return self.raw.get("handle")

    @property
    def display_name(self) -> str | None:
        """SELF-DECLARED. Whatever its owner typed. See `label`."""
        return self.raw.get("display_name")

    @property
    def sworn(self) -> bool:
        return bool(self.raw.get("sworn"))

    @property
    def avatar(self) -> str | None:
        """A path on the origin — join it to the host before fetching."""
        return self.raw.get("avatar")

    @property
    def address(self) -> str | None:
        return self.raw.get("address")

    @property
    def identities(self) -> int:
        """How many vows this wallet holds. One party, several identities."""
        return int(self.raw.get("identities") or 1)

    @property
    def label(self) -> str:
        """The one string safe to draw without thinking about it.

        A sworn name renders bare; anything self-declared gets `~`, the
        town's marker for *this party said so and nobody checked*
        (`ACHIEVEMENTS.md`). A surface that shows a claimed name in the
        clothes of a checked one has lied for free, and it is the cheapest
        mistake to make and the hardest to notice.
        """
        if self.sworn and self.handle:
            return self.handle
        name = self.display_name or ""
        if name:
            return f"~{name}"
        addr = self.address or ""
        return f"{addr[:6]}…{addr[-4:]}" if len(addr) >= 10 else "anonymous"

    def link(self, what: str) -> str | None:
        """A path you fetch yourself — `achievements`, `holdings`,
        `reputation`, `vow`, `tip`. One verb is one origin read, the way Steam
        splits summaries from achievements."""
        return (self.raw.get("links") or {}).get(what)


# ── building messages offline ────────────────────────────────────────────────
# The server's format is deterministic on purpose (`meter/playauth.py`), so a
# game never needs a round trip to know what it is about to ask for. Rebuilt
# here rather than fetched: a client that asks a server what to sign has given
# the server the ability to change what it is signing.

def play_message(action: str, wallet: str, detail: str, day: str | None = None) -> str:
    """The exact text a wallet signs for one game action.

    `day` is UTC and defaults to today. It is a parameter because a round that
    straddles midnight should sign the day it started, and a client that
    cannot express that would silently sign the wrong one.

    ⚠ **The subject is the WALLET, and it was the vow_id until 2026-08-12.** A
    game does not send its players to swear a vow before they can act: the
    signature recovers the signer, so the wallet is the identity and the vow is
    a name you may add later. The server still accepts the old vow-keyed text
    from a caller that has one — build this one.

    ⚠ **The address is LOWERCASE in the text.** A checksummed address is
    different bytes and verifies differently. This is the one detail that will
    bite a game that hand-rolls the string, so it is normalized here.
    """
    import time
    day = day or time.strftime("%Y-%m-%d", time.gmtime())
    return (f"scry play\naction: {action}\nwallet: {(wallet or '').lower()}\n"
            f"day: {day}\ndetail: {detail}")


class Overlay:
    def __init__(self, game: str, version: str = "", path: str | None = None,
                 timeout: float = DEFAULT_TIMEOUT):
        self.game = game
        self.version = version
        self.path = path or default_socket()
        self.timeout = timeout
        self._sock: socket.socket | None = None
        self.hello: dict = {}
        self.why: str = "not connected"

    # ── lifecycle ───────────────────────────────────────────────────────────
    def connect(self) -> bool:
        """True if a launcher answered. False is normal — play anyway."""
        try:
            if os.name == "nt":
                s = _PipeConn(self.path, self.timeout)
            else:
                s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.settimeout(self.timeout)
                s.connect(self.path)
        except (OSError, socket.timeout) as exc:
            self.why = f"no launcher at {self.path}: {exc}"
            return False
        self._sock = s
        got = self._call({"op": "hello", "game": self.game, "protocol": PROTOCOL,
                          "version": self.version})
        if not got.get("ok"):
            self.why = got.get("reason", "the launcher refused hello")
            self.close()
            return False
        self.hello = got
        self.why = ""
        if got.get("protocol") != PROTOCOL:
            # Not fatal by itself, but the game should decide, not this file.
            self.why = (f"the launcher speaks protocol {got.get('protocol')}, "
                        f"this client speaks {PROTOCOL}")
        return True

    @property
    def connected(self) -> bool:
        return self._sock is not None

    @property
    def signer(self) -> str:
        """Which backend holds the key: local · browser · arca · external · none.

        Worth reading BEFORE asking: with `none` there is no point building a
        signable act, and with `browser` a game should expect a handoff rather
        than a signature and shape its UI for that.

        ⚠ **`local` is what the shipped client reports** once the player has
        made an account in it, and it is the only backend `prove` works on.
        The others are reachable by anyone building on the crate; a downloaded
        launcher answers `local` or `none` and nothing else.
        """
        return str(self.hello.get("signer") or "none")

    def close(self):
        if self._sock:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None

    def __enter__(self):
        self.connect()
        return self

    def __exit__(self, *_exc):
        self.close()

    # ── the verbs ───────────────────────────────────────────────────────────
    def address(self) -> str | None:
        """The address the player asked their launcher to watch.

        A CLAIM, not authentication — anything can say a number. If it matters,
        ask for a signature over something you chose and recover the signer.
        """
        got = self._call({"op": "identity"})
        return got.get("address") if got.get("ok") else None

    def sign(self, text: str, why: str = "") -> Result:
        req = {"op": "sign", "text": text}
        if why:
            req["why"] = why
        got = self._call(req)
        return Result(
            ok=bool(got.get("ok")), signature=got.get("signature"),
            address=got.get("address"), family=got.get("family", ""),
            backend=got.get("backend", ""), reason=got.get("reason", ""),
            handoff=got.get("handoff"),
            refused_by_player=got.get("refused_by") == "the player", raw=got)

    def prove(self, server: str, nonce: str) -> Result:
        """**Prove to your own server who is playing** — the verb behind a
        ticket check, and the one to reach for instead of `address()`, which is
        a claim anything can make.

        The reply is a **SIWE (EIP-4361) message and signature**, so your
        backend needs no scry-specific code::

            from siwe import SiweMessage
            m = SiweMessage.from_message(body["message"])
            m.verify(signature=body["signature"],
                     nonce=the_nonce_you_issued,
                     domain="shard-3.gates.example")
            # m.address is now proven. Look it up and admit or kick.

        `siwe` ships for JS, Python, Rust and Go, and viem and ethers have it
        built in.

        `server` is your **domain** — `shard-3.gates.example`, optionally with
        `:port`. No scheme, no path: EIP-4361 binds the domain, and the
        launcher writes the `URI:` line itself.

        ⚠ **The nonce must be one you issued and have not seen before**, and
        EIP-4361 requires **at least 8 alphanumeric characters**. A dashed uuid
        is refused — strip the dashes. A signature over a message with no fresh
        nonce is valid forever to anyone who captures it, so bind it to the
        connection and retire it on use.

        ⚠ **Do not recompute the message string and compare.** It carries
        `Issued At` from the launcher's clock, so you cannot rebuild it. Parse
        it and check the fields you care about — which is what `verify` above
        does, and why passing your own `nonce` and `domain` to it is the whole
        check. Never accept the `message` field without doing that.

        **No consent prompt fires**, by construction rather than by permission:
        the launcher composes every word, so a game cannot smuggle a sentence
        into an unprompted signature. `sign` — where the game writes the text —
        still asks the player.
        """
        got = self._call({"op": "prove", "server": server, "nonce": nonce})
        return Result(
            ok=bool(got.get("ok")), signature=got.get("signature"),
            address=got.get("address"), family="prove",
            backend=got.get("backend", ""), reason=got.get("reason", ""),
            handoff=None, refused_by_player=False, raw=got)

    def profile(self, address: str = "") -> "Profile | None":
        """**A name and a face to draw** — Steam's `GetPlayerSummaries`.

        Defaults to the address this launcher watches; pass one to look up
        somebody else (the address you got back from `prove`, say, or another
        player on your shard).

        ⚠ **Three states, and a game that collapses them tells a lie.**
        `None` means *we could not look* — the origin was unreachable, or this
        launcher has no reader. A `Profile` with `sworn=False` and no `handle`
        means *we looked and there is no sworn identity*, which is a normal way
        to play. Drawing "no name" for the first invents a fact about a
        stranger. Check `reachable` on the raw reply if you need to tell a
        network problem from an anonymous player.

        ⚠ **`display_name` is self-declared and `handle` is not.** The handle
        is in the locked index and cost SCRY to change; the display name is
        whatever its owner typed. Draw them differently — the town's
        convention is a `~` prefix on the unchecked one — or draw only the
        handle.

        ⚠ **This is not authentication.** It describes an address; it does not
        establish that the player is holding its key. Call `prove` for that,
        then look up the address you recovered.
        """
        got = self._call({"op": "profile", "address": address} if address
                         else {"op": "profile"})
        if not got.get("ok"):
            return None
        p = got.get("profile")
        return Profile(raw=p, reachable=bool(got.get("reachable", True))) if p is not None \
            else Profile(raw={}, reachable=bool(got.get("reachable", True)), found=False)

    def title(self, slug: str) -> str | None:
        """The URL of this title's **manifest**, which YOU fetch.

        ⚠ It returns a url, not a title object — the launcher does not proxy a
        game's own documents, the same rule `servers_url` states. This method
        returned `got["title"]` until 2026-08-07 and therefore answered `None`
        for every slug against the real broker, which has always replied with
        `url`. It went unnoticed because this client was only ever driven
        against a second broker that no longer exists.
        """
        got = self._call({"op": "title", "slug": slug})
        return got.get("url") if got.get("ok") else None

    def servers_url(self, slug: str) -> str | None:
        """The URL of the shard list, which YOU fetch. The launcher does not
        proxy it — a launcher between a game and its own server list is a
        cache nobody asked for and a ranking nobody can see."""
        got = self._call({"op": "servers", "slug": slug})
        return got.get("url") if got.get("ok") else None

    def overlay(self, slug: str = "") -> dict | None:
        """One snapshot a game can DRAW — see the Rust client's `overlay` for
        the full note. Display freely; never approve. There is no consent
        token here and the launcher will not accept one back.
        """
        reply = self._call({"op": "overlay", "slug": slug or self.game})
        return reply.get("overlay") if reply.get("ok") else None

    def open_url(self, url: str) -> bool:
        return bool(self._call({"op": "open", "url": url}).get("ok"))

    # ── the wire ────────────────────────────────────────────────────────────
    def _call(self, req: dict) -> dict:
        if self._sock is None:
            return {"ok": False, "reason": self.why or "not connected to a launcher"}
        try:
            self._sock.sendall((json.dumps(req) + "\n").encode("utf-8"))
            buf = b""
            while b"\n" not in buf:
                chunk = self._sock.recv(4096)
                if not chunk:
                    self.close()
                    return {"ok": False, "reason": "the launcher closed the connection"}
                buf += chunk
        except (OSError, socket.timeout) as exc:
            self.close()
            return {"ok": False, "reason": f"lost the launcher: {exc}"}
        try:
            return json.loads(buf.split(b"\n", 1)[0].decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as exc:
            return {"ok": False, "reason": f"the launcher replied with non-JSON: {exc}"}


if __name__ == "__main__":                                  # a live smoke check
    import sys
    ov = Overlay(game="scry-overlay-demo", version="1")
    if not ov.connect():
        print(ov.why)
        print("\nThat is a normal state. A game plays without a launcher; what it "
              "must never do is ask the player for a private key instead.")
        raise SystemExit(1)
    print(json.dumps({"hello": ov.hello, "address": ov.address()}, indent=2))
    if len(sys.argv) > 1 and sys.argv[1] == "--sign":
        out = ov.sign(play_message("answer", "vow_demo", "a" * 8))
        print(json.dumps(out.raw, indent=2))
    ov.close()
