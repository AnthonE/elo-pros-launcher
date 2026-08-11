#!/usr/bin/env python3
"""Verify Rust-signed Nostr events with the METER'S OWN verifier.

The Rust client is a second implementation of a cryptographic serialization —
NIP-01's event id and BIP-340 over it — and the only honest check that it
produces what the relay accepts is to run the server's code over its output.
`meter/hive.py:verify_event` is that code.

    cargo test -p scry-hive emit_events_for_the_python_verifier
    python3 crates/scry-hive/tests/parity_verify.py

Same discipline as `scry-depot`'s digest parity, and for the same reason: a
signature that is subtly wrong is refused silently by the relay, which is
exactly the failure mode this whole layer exists to avoid.
"""
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "meter"))

try:
    from hive import verify_event, npub_encode  # noqa: E402
except Exception as exc:                        # pragma: no cover
    print(f"NOT RUN — could not import meter/hive.py: {exc}")
    print("This is not a pass. The Rust signer is unchecked without it.")
    sys.exit(2)

payload_path = ROOT / "launcher-rs/target/nostr_parity.json"
if not payload_path.is_file():
    print(f"NOT RUN — {payload_path} is missing.")
    print("Run: cargo test -p scry-hive emit_events_for_the_python_verifier")
    sys.exit(2)

payload = json.loads(payload_path.read_text("utf-8"))
events = payload["events"]
failures = 0

print(f"verifying {len(events)} Rust-signed events with meter/hive.py")
print(f"  npub {payload['npub']}")

for ev in events:
    problem = verify_event(ev)
    label = (ev["content"] or "<empty>")[:44].replace("\n", "\\n")
    if problem:
        print(f"  FAIL  kind:{ev['kind']:<3} {label!r}: {problem}")
        failures += 1
    else:
        print(f"  ok    kind:{ev['kind']:<3} {label!r}")

# The npub the Rust side printed must be the one the meter derives from the
# event's own pubkey — otherwise the key a person copies out of the client
# addresses someone else.
try:
    derived = npub_encode(bytes.fromhex(payload["pubkey"]))
    if derived != payload["npub"]:
        print(f"  FAIL  npub mismatch: rust {payload['npub']} vs meter {derived}")
        failures += 1
    else:
        print(f"  ok    npub encoding matches the meter's")
except Exception as exc:
    print(f"  FAIL  could not derive an npub: {exc}")
    failures += 1

print()
if failures:
    print(f"{failures} FAILED — the Rust signer disagrees with the server.")
    sys.exit(1)
print(f"all {len(events)} events verify under the meter's own BIP-340 check.")
