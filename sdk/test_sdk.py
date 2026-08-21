#!/usr/bin/env python3
"""The SDK's laws — the Python client, against the real broker.

    python3 sdk/test_sdk.py

There is **one broker** and it is Rust (`launcher-rs/crates/elo-broker`). This
suite drives the **Python SDK client** against it over a real UNIX socket, which
is the only cross-language surface left in the protocol and therefore the only
place the two halves can drift.

## What changed, and why this file shrank

Until 2026-08-07 there were two brokers — this suite stood up the Python one and
`sdk_parity.rs` stood up the Rust one, and **nothing compared them.** That gap
shipped `prove` in the Rust broker, both SDK clients and `sdk/PROTOCOL.md` while
the Python broker refused it as an unknown op, with every suite green. The fix
was a parity check; the operator's fix was better (*"delete the python launcher
we dont use it anymore"*), because a class of bug that needs a test to catch is
worse than a class of bug that cannot exist.

So most of what used to be here was testing a broker that is gone. What remains
is what still has two sides: **this client and that server.**

The Rust half of the client is covered by `crates/elo-broker/tests/sdk_parity.rs`,
which compiles the vendored `sdk/rust/elo_overlay.rs` and drives it against the
same server. Neither suite replaces the other.

⚠ **This needs `cargo`.** If the broker cannot be built the suite prints NOT RUN
on its own line and the tail repeats it, because a suite that folds a skip into
a green count is worse than no suite (`CLAUDE.md` §traps).
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
sys.path.insert(0, str(HERE / "python"))

import elo_overlay  # noqa: E402

passed = failed = 0
skipped: list[str] = []
failures: list[str] = []


def check(name, cond, detail=""):
    global passed, failed
    if cond:
        passed += 1
    else:
        failed += 1
        failures.append(f"{name}{(' — ' + detail) if detail else ''}")
        print(f"  FAIL  {name}" + (f"  — {detail}" if detail else ""))


def section(title):
    print(f"\n── {title} " + "─" * max(0, 56 - len(title)))


# ── the server under test ───────────────────────────────────────────────────

RS = REPO / "launcher-rs"
BROKER = RS / "target" / "debug" / "examples" / "headless"


def build_broker() -> str:
    """Build the headless broker, or say why not. Never returns a stale binary
    silently — a suite driving last week's server is the thing this file exists
    to prevent."""
    try:
        out = subprocess.run(
            ["cargo", "build", "--example", "headless", "-p", "elo-broker"],
            cwd=RS, capture_output=True, text=True, timeout=900)
    except (FileNotFoundError, subprocess.TimeoutExpired) as exc:
        return f"cargo could not run ({type(exc).__name__})"
    if out.returncode != 0:
        return f"the broker did not build: {out.stderr.strip()[-400:]}"
    if not BROKER.is_file():
        return f"no binary at {BROKER}"
    return ""


class Broker:
    """One headless broker on its own socket, for the life of a `with` block."""

    def __init__(self, refuse_consent=False):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "launcher.sock")
        self.proc = None
        self.refuse = refuse_consent

    def __enter__(self):
        argv = [str(BROKER), self.path]
        if self.refuse:
            argv.append("--refuse-consent")
        self.proc = subprocess.Popen(argv, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, text=True)
        # Wait for the bind, never for the file to appear — the server prints
        # `ready` only after `bind` returned, and racing on os.path.exists is
        # how a flaky integration test is born.
        line = self.proc.stdout.readline()
        if not line.startswith("ready"):
            raise RuntimeError(f"broker did not come up: {line!r}")
        return self

    def client(self, game="gates", version="1.0.0"):
        ov = elo_overlay.Overlay(game, version, path=self.path)
        ov.connect()
        return ov

    def __exit__(self, *_exc):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        self.dir.cleanup()


why_not = build_broker()

if why_not:
    skipped.append(f"the whole protocol — {why_not}")
else:
    # ═══ the handshake ══════════════════════════════════════════════════════
    section("the handshake")
    with Broker() as br:
        ov = br.client()
        check("the client connects to the Rust broker", ov.connected)
        check("hello names which signer is configured", ov.signer == "local", ov.signer)
        check("a game that never says hello gets nothing",
              elo_overlay.Overlay("x", path=br.path)._call({"op": "identity"})
              .get("ok") is False)
        # The vocabulary is the whole of a game's authority.
        caps = ov._call({"op": "hello", "game": "gates", "protocol": 1}).get("capabilities")
        check("hello advertises the vocabulary", isinstance(caps, list) and caps, repr(caps))
        for absent in ("install", "uninstall", "launch", "read_file", "settings",
                       "set_signer", "quit", "grant"):
            r = ov._call({"op": absent})
            check(f"there is no {absent} verb",
                  r["ok"] is False and "unknown op" in r["reason"])
        r = ov._call({"op": "identity", "wallet": "0xdead"})
        check("an unknown field is refused, not ignored",
              r["ok"] is False and "unknown field" in r["reason"], repr(r))
        check("identity answers an address",
              (ov.address() or "").startswith("0x"), repr(ov.address()))
        ov.close()

    # ═══ prove — SIWE, and the client parses what the server composes ═══════
    section("prove — sign-in with ethereum")
    with Broker() as br:
        ov = br.client()
        NONCE, DOMAIN = "8f14e45fceea167a", "shard-3.gates.example"
        r = ov.prove(DOMAIN, NONCE)
        check("prove returns a signature", r.ok and r.signature, repr(r.reason))
        msg = (r.raw or {}).get("message") or ""
        check("the reply names the scheme",
              (r.raw or {}).get("scheme") == "eip4361", repr((r.raw or {}).get("scheme")))
        check("the domain leads — that is what binds a proof to one server",
              msg.startswith(f"{DOMAIN} wants you to sign in with your Ethereum account:\n"))
        check("the address the client reports is the address in the message",
              (r.address or "") in msg, f"{r.address} not in message")
        check("the game names itself in the statement",
              "\nProve your identity to play gates." in msg)
        check("the chain is bound", "\nChain ID: 4663\n" in msg)
        check("the nonce the server issued is in the signed bytes",
              f"\nNonce: {NONCE}\n" in msg)
        check("the launcher builds the URI itself",
              f"\nURI: https://{DOMAIN}\nVersion: 1\n" in msg)
        check("Issued At is ISO-8601 UTC",
              any(l.startswith("Issued At: ") and l.endswith("Z") and "T" in l
                  for l in msg.split("\n")), repr(msg.split("\n")[-1]))
        check("the reply points at siwe rather than hand-rolled parsing",
              "siwe" in (r.raw or {}).get("verify", ""))

        # ⚠ EIP-4361's alphabet, and it is stricter than a hand-rolled guard.
        for bad, expect in (("550e8400-e29b-41d4-a716-446655440000", "alphanumeric"),
                            ("abc_-123+/=", "alphanumeric"),
                            ("short7", "at least 8"),
                            ("nonce\nNonce: other", "alphanumeric"),
                            ("", "empty")):
            rr = ov.prove(DOMAIN, bad)
            check(f"a nonce that is {expect!r}-refused: {bad[:20]!r}",
                  rr.ok is False and expect in rr.reason, repr(rr.reason)[:90])
        check("a stripped uuid is fine, which is the documented fix",
              ov.prove(DOMAIN, "550e8400e29b41d4a716446655440000").ok)
        # The server field is a DOMAIN — it lands in the signed bytes twice.
        for bad, expect in (("https://s.example", "not a url"),
                            ("s.example/play", "not a url"),
                            ("s.example:notaport", "port"),
                            ("bad host.example", "not a hostname"),
                            ("a\nb", "not a hostname")):
            rr = ov.prove(bad, NONCE)
            check(f"a server that is {expect!r}-refused: {bad[:20]!r}",
                  rr.ok is False and expect in rr.reason, repr(rr.reason)[:90])
        check("a port is legitimate — a shard on 7777 is ordinary",
              ov.prove("shard-3.gates.example:7777", NONCE).ok)
        ov.close()

    # **No consent prompt fires for prove**, and that is the whole verb. Proved
    # against a server whose player refuses everything.
    with Broker(refuse_consent=True) as br:
        ov = br.client()
        check("sign is refused by this server",
              ov.sign(elo_overlay.play_message("duel", "v", "x"), "why").ok is False)
        check("prove still works — it asks nobody",
              ov.prove("s.example", "8f14e45fceea167a").ok)
        ov.close()

    # ═══ profile — a name and a face ════════════════════════════════════════
    section("profile — a name and a face")
    with Broker() as br:
        ov = br.client()
        p = ov.profile("")
        check("profile answers for the watched address", p is not None and p.found)
        check("a sworn name and a typed one stay separate fields",
              all(k in p.raw for k in ("handle", "display_name", "sworn")))
        check("every link carries the /api prefix the edge strips",
              all(v is None or v.startswith("/api/")
                  for v in (p.raw.get("links") or {}).values()),
              repr(p.raw.get("links")))
        check("the avatar is a path a game can fetch",
              (p.raw.get("avatar") or "/api/").startswith("/api/"))
        check("identities carries the count through — one party, several vows",
              p.identities >= 1, repr(p.identities))
        # ⚠ Three states, not two.
        dead = ov.profile("0x000000000000000000000000000000000000dEaD")
        check("a looked-up-and-absent identity is FOUND=False, not None",
              dead is not None and dead.found is False, repr(dead))
        check("…and it carries no name rather than a blank one",
              dead is not None and dead.handle is None)
        for bad in ("nope", "0x123", "0x" + "zz" * 20):
            rb = ov._call({"op": "profile", "address": bad})
            check(f"a malformed address {bad!r} is refused before any url is built",
                  rb["ok"] is False and "40 hex" in rb.get("reason", ""), repr(rb)[:90])
        ov.close()

    # ═══ the rest of the wire ═══════════════════════════════════════════════
    section("the rest of the wire")
    with Broker() as br:
        ov = br.client()
        check("title answers for a slug", ov.title("gates") is not None)
        check("servers answers None when a title publishes no list — a real state",
              ov.servers_url("gates") is None)
        hud = ov.overlay("gates")
        check("overlay returns a snapshot a HUD can draw", isinstance(hud, dict), repr(hud))
        # ⚠ Display freely; never approve. Asserted BY KEY, because the consent
        # note legitimately contains the word "approve" while explaining that
        # nothing may.
        for forbidden in ("token", "approve", "allow", "pending", "grant"):
            check(f"the overlay carries no {forbidden!r} key a game could act on",
                  forbidden not in (hud or {}), repr(sorted(hud or {})))
        check("open refuses anything that is not https",
              ov._call({"op": "open", "url": "file:///etc/passwd"})["ok"] is False)
        ov.close()

    # ═══ the client's own care ══════════════════════════════════════════════
    section("the client's own care")
    P = elo_overlay.Profile
    check("a sworn handle draws bare",
          P(raw={"handle": "moss", "sworn": True}).label == "moss")
    check("⚠ a self-declared name is marked, never drawn as a checked one",
          P(raw={"display_name": "moss", "sworn": False}).label == "~moss")
    check("…even when a handle is present but not sworn",
          P(raw={"handle": "moss", "display_name": "moss", "sworn": False}).label == "~moss")
    check("a nameless player falls back to a short address",
          P(raw={"address": "0x" + "ab" * 20}).label == "0xabab…abab")
    check("and to a word when there is not even that", P(raw={}).label == "anonymous")
    check("links are read through, not invented",
          P(raw={"links": {"achievements": "/api/x"}}).link("achievements") == "/api/x"
          and P(raw={}).link("achievements") is None)
    check("found=False carries no name", P(raw={}, found=False).handle is None)

    # `play_message` is built OFFLINE. A client that asks a server what to sign
    # has handed that server the ability to change what it is signing.
    # ⚠ Keyed to the WALLET since 2026-08-12, and the address is lowercased for
    # you — a checksummed one is different bytes and would verify differently.
    m = elo_overlay.play_message("duel", "0xAbC0000000000000000000000000000000000001",
                                  "ETH up 5", day="2026-08-07")
    check("play_message is deterministic and rebuildable offline",
          m == ("elo play\naction: duel\n"
                "wallet: 0xabc0000000000000000000000000000000000001\n"
                "day: 2026-08-07\ndetail: ETH up 5"),
          repr(m))
    check("…and it names its family on the first line, which is what consent counts",
          m.split("\n")[0] == "elo play")

    # ═══ the spec is what a third party reads ═══════════════════════════════
    section("the spec is what a third party reads")
    proto = (HERE / "PROTOCOL.md").read_text()
    with Broker() as br:
        ov = br.client()
        caps = ov._call({"op": "hello", "game": "gates", "protocol": 1})["capabilities"]
        ov.close()
    # A verb the broker serves that the spec does not document is undiscoverable;
    # one the spec documents that no broker serves is a lie. This is the check
    # that would have caught `prove` being dark, and it survives the collapse to
    # one broker because the SPEC is still a second implementation of the truth.
    for verb in caps:
        if verb == "hello":
            continue
        check(f"PROTOCOL.md documents {verb!r}", f"`{verb}`" in proto)
    pysrc = (HERE / "python" / "elo_overlay.py").read_text()
    rssrc = (HERE / "rust" / "elo_overlay.rs").read_text()
    for verb in caps:
        if verb == "hello":
            continue
        check(f"the Python client can call {verb!r}", f'"op": "{verb}"' in pysrc)
        check(f"the Rust client can call {verb!r}", f'Field::S("{verb}")' in rssrc)

    # The golden SIWE bytes. There is one implementation now (Rust), so this is
    # a frozen regression pin rather than a parity check — but it is pinned on
    # BOTH sides on purpose: `crates/elo-broker/tests/cross_language.rs` asserts
    # the composer, and this asserts what actually came off the wire.
    fx = json.loads((HERE / "testdata" / "prove_fixture.json").read_text())
    with Broker() as br:
        ov = br.client()
        got = (ov.prove(fx["domain"], fx["nonce"]).raw or {}).get("message", "")
        ov.close()
    live = [l for l in got.split("\n") if not l.startswith("Issued At: ")]
    want = [l for l in fx["message"].split("\n") if not l.startswith("Issued At: ")]
    # The address differs (the fixture's is the origin's, the server's is its
    # test key), so compare the shape and every field a verifier gates on.
    check("the live SIWE message has the fixture's exact line shape",
          len(live) == len(want) and [bool(l) for l in live] == [bool(l) for l in want],
          f"{live}")
    for i, line in enumerate(want):
        if line.startswith(("URI:", "Version:", "Chain ID:", "Nonce:")):
            check(f"the live message matches the fixture at {line.split(':')[0]!r}",
                  live[i] == line, f"{live[i]!r} vs {line!r}")


# ═══ what a VENDORING game checks against ═══════════════════════════════════
#
# These need no broker and no cargo, so they run even when the section above
# is skipped — which is deliberate, because the defect they guard is one a
# vendoring repo discovers and this one cannot.
#
# The story: Gates vendors `rust/elo_overlay.rs` byte-for-byte and pins it
# with a sha256. That pin catches a LOCAL EDIT and is blind to upstream moving,
# so on 2026-08-09 its copy was found 326 lines behind this one — no Windows
# transport, no `prove`, no `profile` — with every gate in both repos green.
# `crates/elo-broker/tests/sdk_parity.rs` even calls this file "what Gates has
# compiled into its binary byte-for-byte", which was a claim about another
# repo that nothing checked.
#
# Nothing here can check another repo either. What it can do is make the drift
# check ONE COMMAND against a number we publish — `sha256sum` the vendored
# file, look for it in `sdk/SHA256SUMS` — and keep the two properties a vendor
# actually needs from breaking silently.
section("what a vendoring game checks against")

sums_path = HERE / "SHA256SUMS"
check("sdk/SHA256SUMS is committed", sums_path.is_file(),
      "a vendoring game has nothing to compare its copy against without it")
if sums_path.is_file():
    import hashlib

    published = {}
    for line in sums_path.read_text().splitlines():
        if line.strip():
            digest, name = line.split(None, 1)
            published[name.strip()] = digest
    for rel in ("rust/elo_overlay.rs", "python/elo_overlay.py"):
        actual = hashlib.sha256((HERE / rel).read_bytes()).hexdigest()
        check(f"SHA256SUMS names {rel}", rel in published)
        check(f"…and matches the bytes we ship for {rel}",
              published.get(rel) == actual,
              f"published {published.get(rel)!r} vs actual {actual!r} — "
              f"regenerate: cd sdk && sha256sum rust/elo_overlay.rs "
              f"python/elo_overlay.py > SHA256SUMS")

# A vendoring project runs its own fmt gate over everything in its tree,
# including this file. It has happened twice that an unformatted SDK reddened
# a consumer's CI, and nothing upstream was watching for it — `launcher-rs` has
# no fmt gate of its own, so this is the only place the property lives.
rs_path = HERE / "rust" / "elo_overlay.rs"
try:
    fmt = subprocess.run(["rustfmt", "--edition", "2021", "--check", str(rs_path)],
                         capture_output=True, text=True, timeout=60)
except (OSError, subprocess.TimeoutExpired) as e:
    skipped.append(f"the rustfmt check — rustfmt did not run ({e.__class__.__name__})")
else:
    check("the Rust SDK is rustfmt-clean, so a vendoring project's fmt gate passes",
          fmt.returncode == 0 and not fmt.stdout.strip(),
          (fmt.stdout or fmt.stderr).strip()[:400])

# The transport is the one thing in this file that is platform-specific, and
# an unguarded `use std::os::unix::net` does not fail HERE — it fails in a
# game's Windows build, which is where nobody is looking. That is the exact
# shape the 2026-08-09 drift had.
rs_src = rs_path.read_text()
for line in rs_src.splitlines():
    if line.strip().startswith("use std::os::unix"):
        idx = rs_src.splitlines().index(line)
        guarded = any("cfg(unix)" in rs_src.splitlines()[j]
                      for j in range(max(0, idx - 3), idx))
        check("every unix-only import is behind cfg(unix)", guarded, line.strip())
check("and a windows transport exists at all", "cfg(windows)" in rs_src,
      "a game vendoring this must be able to build for Windows")


# ═══ tail ═══════════════════════════════════════════════════════════════════
print()
print("═" * 62)
print(f"  {passed} passed, {failed} failed")
if skipped:
    print(f"  {len(skipped)} SECTION(S) NOT RUN — this is not a pass:")
    for s in skipped:
        print(f"    · {s}")
else:
    print("  every section ran")
if failures:
    print("\n  failures:")
    for f in failures:
        print(f"    · {f}")
print("═" * 62)
raise SystemExit(1 if failed else 0)
