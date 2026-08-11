#!/usr/bin/env python3
"""sign_manifest.py — sign a game's `scry.json` so scry will read it.

Copy this into your game's repo. It has one dependency (`eth-account`) and it
talks to nothing: it hashes a file, builds the exact text scry checks, signs it
with a key **you** hold, and writes `scry.sig.json` beside the manifest.

    pip install eth-account
    python3 sign_manifest.py --game gates                 # bump seq, sign, write
    python3 sign_manifest.py --game gates --check         # verify, no key needed
    python3 sign_manifest.py --game gates --print         # the text; sign it elsewhere

Where the key comes from, in the order tried:

    --key 0x…              a hex private key on the command line (fine in CI
                           from a secret; it is in your shell history otherwise)
    SCRY_DEV_KEY           the same, from the environment. This is the CI shape.
    --keystore <file>      an encrypted V3 keystore; the passphrase is prompted
                           for, or read from SCRY_DEV_PASSPHRASE
    --print                no key at all. Prints the message; paste the
                           signature back with --signature. This is the path for
                           a hardware wallet, and it is the one scry itself
                           would take — the service that reads this file holds
                           no key either.

The seq is the anti-replay counter and it only goes up. With no `--seq` this
reads the existing `scry.sig.json` and adds one, which is right unless the last
signature was never applied — ask the origin what it will accept:

    curl -s https://scry.moreright.xyz/api/store/repo/<slug> | grep next_seq

Unlike `scry_overlay.rs`, this file carries NO checksum pin and is not part of
any protocol. Edit it, rewrite it in another language, fold it into your release
job — the only things scry checks are the two files it produces.
"""
from __future__ import annotations

import argparse
import getpass
import hashlib
import json
import os
import sys
from pathlib import Path

MANIFEST_NAME = "scry.json"
SIG_NAME = "scry.sig.json"


def message(game: str, digest: str, seq: int) -> str:
    """The exact text scry recovers a signer from. Byte-for-byte — a stray
    space makes a signature that verifies to a different address."""
    return (f"scry repo\ngame: {game}\nmanifest: {digest}\nseq: {seq}\n"
            f"by signing, this wallet publishes this manifest as '{game}' on scry, "
            f"under its own address, in public.")


def die(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    raise SystemExit(1)


def load_account(args):
    """The key, or None when we are only printing a message to sign elsewhere."""
    try:
        from eth_account import Account
    except ImportError:
        die("eth-account is not installed — `pip install eth-account`, or use "
            "--print and sign the text with whatever holds your key")
    if args.key or os.getenv("SCRY_DEV_KEY"):
        return Account.from_key(args.key or os.environ["SCRY_DEV_KEY"])
    if args.keystore:
        blob = json.loads(Path(args.keystore).read_text(encoding="utf-8"))
        pw = os.getenv("SCRY_DEV_PASSPHRASE") or getpass.getpass("keystore passphrase: ")
        return Account.from_key(Account.decrypt(blob, pw))
    return None


def sign(account, text: str) -> str:
    from eth_account.messages import encode_defunct
    sig = account.sign_message(encode_defunct(text=text)).signature.hex()
    return sig if sig.startswith("0x") else "0x" + sig


def recover(text: str, signature: str) -> str:
    from eth_account import Account
    from eth_account.messages import encode_defunct
    return Account.recover_message(encode_defunct(text=text), signature=signature)


def main() -> int:
    ap = argparse.ArgumentParser(description="sign a scry game manifest")
    ap.add_argument("--game", help="the slug, as scry already lists it. "
                                   "Defaults to the manifest's own \"game\" field.")
    ap.add_argument("--manifest", default=MANIFEST_NAME)
    ap.add_argument("--out", default=SIG_NAME)
    ap.add_argument("--seq", type=int, help="the counter. Default: the current "
                                            "signature's seq plus one, or 1.")
    ap.add_argument("--key", help="a hex private key")
    ap.add_argument("--keystore", help="an encrypted V3 keystore file")
    ap.add_argument("--signature", help="a signature produced elsewhere, to write out")
    ap.add_argument("--wallet", help="the address that made --signature")
    ap.add_argument("--print", dest="show", action="store_true",
                    help="print the text to sign and exit — no key is touched")
    ap.add_argument("--check", action="store_true",
                    help="verify the committed pair and exit — no key is touched")
    args = ap.parse_args()

    path = Path(args.manifest)
    if not path.is_file():
        die(f"{path} is not here. Start from the template in scry's "
            f"sdk/repo/{MANIFEST_NAME}.")
    raw = path.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()

    try:
        parsed = json.loads(raw.decode("utf-8"))
    except Exception as e:  # noqa: BLE001
        die(f"{path} is not readable JSON: {e}. scry will refuse it for the same reason.")
    game = args.game or str(parsed.get("game") or "")
    if not game:
        die("no slug — pass --game, or set \"game\" in the manifest")
    if parsed.get("game") and parsed["game"] != game:
        die(f"the manifest says \"game\": {parsed['game']!r} and you passed {game!r}. "
            f"scry refuses a manifest that names a title other than the one it was "
            f"fetched from, so fix one of the two.")

    out = Path(args.out)
    prior = {}
    if out.is_file():
        try:
            prior = json.loads(out.read_text(encoding="utf-8"))
        except Exception:  # noqa: BLE001 — a torn signature file reads as absent
            prior = {}

    if args.check:
        if not prior:
            die(f"{out} is missing or unreadable — nothing to check")
        claimed = str(prior.get("sha256") or "")
        ok_hash = claimed == digest
        print(f"manifest : {path} ({len(raw)} bytes)")
        print(f"sha256   : {digest}")
        print(f"claimed  : {claimed or '(absent)'}  {'OK' if ok_hash else 'MISMATCH'}")
        if not ok_hash:
            die("the signature file was written for different bytes — re-sign "
                "and commit both files together")
        text = message(game, digest, int(prior.get("seq") or 0))
        try:
            got = recover(text, str(prior.get("signature") or ""))
        except Exception as e:  # noqa: BLE001
            die(f"the signature does not recover: {e}")
        want = str(prior.get("wallet") or "")
        print(f"seq      : {prior.get('seq')}")
        print(f"recovers : {got}")
        if got.lower() != want.lower():
            die(f"it recovers {got}, and the file claims {want}")
        print("OK — this pair verifies. scry will accept it if that wallet has "
              "standing on the slug and the seq has not been used.")
        return 0

    seq = args.seq if args.seq is not None else int(prior.get("seq") or 0) + 1
    if seq < 1:
        die("seq starts at 1 and only goes up")
    text = message(game, digest, seq)

    if args.show:
        print(text)
        print(f"\n# sha256({path}) = {digest}\n# seq = {seq}", file=sys.stderr)
        print(f"# sign the text above, then:\n#   python3 {Path(__file__).name} "
              f"--game {game} --seq {seq} --signature 0x… --wallet 0x…", file=sys.stderr)
        return 0

    if args.signature:
        if not args.wallet:
            die("--signature needs --wallet: the address that made it")
        got = recover(text, args.signature)
        if got.lower() != args.wallet.lower():
            die(f"that signature recovers {got}, not {args.wallet}. It was probably "
                f"made over a different seq or a different version of the manifest.")
        wallet, signature = args.wallet, args.signature
    else:
        account = load_account(args)
        if account is None:
            die("no key. Pass --key, set SCRY_DEV_KEY, use --keystore, or use "
                "--print and sign the text with whatever holds your key.")
        wallet, signature = account.address, sign(account, text)

    out.write_text(json.dumps({
        "wallet": wallet, "seq": seq, "sha256": digest, "signature": signature,
    }, indent=1) + "\n", encoding="utf-8")

    print(f"wrote {out}")
    print(f"  game      {game}")
    print(f"  seq       {seq}")
    print(f"  sha256    {digest}")
    print(f"  wallet    {wallet}")
    print(f"\ncommit {path} and {out} together — the signature covers those exact bytes.")
    print(f"then:  curl -s https://scry.moreright.xyz/api/store/repo/{game}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
