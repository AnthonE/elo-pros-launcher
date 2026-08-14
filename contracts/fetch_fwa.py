"""fetch_fwa.py — pull the FWA deployment into a reference folder, with provenance.

`DEPLOYMENT` below is every FWA address we know on Ethereum mainnet: the eight
read off https://www.fwa.fun/docs/deployments, plus Token Packs
and its renderer, which are on no page of theirs and were found from a mint in
the wild. This script fetches what is actually published for each one and
writes it under `contracts/reference/fwa/`, so the port is read from CODE rather
than from a paraphrase of their docs.

**This folder is reference, never a dependency.** Nothing here is compiled:
`foundry.toml` builds `src`, `test`, `script` and this is none of them. Nothing
here is imported by `src/`. Two reasons, and the second is the one that matters:

  1. It is another chain's deployment — different randomness (Chainlink VRF
     2.5, absent on 4663), different price floor, different token. Most of
     their pricing is what breaks on the port to 4663.
  2. **Their verified source declares no license** (`license_type: none` on
     every verified file, no license statement on their docs page). Read it,
     measure it, and re-derive our own — do not lift it into `src/`. That is the
     same standing rule `RUNBOOK.md` §6 applies to every borrowed contract, and it is
     cheap to keep here because the mechanic is arithmetic, not code: the
     harmonic-mean weighting and the settlement branch are a page of algebra
     once you can see them, and seeing them is what this folder is for.

**Half the deployment is not verified anywhere.** Measured 2026-07-26: FWARewards,
FWAToken, FWATokenHook and FWAVRFService are verified on Blockscout; **the core
FWA contract, FWAClaim, FWAWhitelist and Splitter are not** — not on
eth.blockscout.com, not in Sourcify (404), and Etherscan's V2 API needs a key
this box does not have. For those, this script writes the deployed bytecode and
reconstructs the **external surface**: every 4-byte selector and 32-byte event
topic embedded in the code, resolved against keyless public signature databases.
That is not source, and the output says so on every line — but a function list
with names is enough to check a design claim against the deployment, which is
the whole job.

Usage:

    python3 contracts/fetch_fwa.py              # everything public
    python3 contracts/fetch_fwa.py --no-sigs    # skip the lookups
    ETHERSCAN_API_KEY=... python3 contracts/fetch_fwa.py   # or source /data/apps/secrets/keys.env

Read-only. Touches no key, signs nothing, spends nothing.
"""
from __future__ import annotations

import argparse

import json
import os
import pathlib
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
# The SCRIPT is tracked; its OUTPUT is not. `.gitignore` carries
# `contracts/reference/` deliberately, because the Uniswap v4-core files that
# arrive inside FWA's verified sources are BUSL-1.1 and must never enter this
# repo. Keeping the recipe in git and the fetched third-party source out of it is
# what makes the provenance rerunnable without vendoring anyone else's licence.
OUT = HERE / "reference" / "fwa"

# From https://www.fwa.fun/docs/deployments, re-read 2026-07-26. Ethereum
# mainnet (chain 1) — none of these has code on RH-Chain.
DEPLOYMENT = {
    "FWA": "0xB276F62DB0ce8CA2Ca5bc522695bE604521eAc1c",
    "FWARewards": "0x6a1a1C0CfB3D3C538e13D36d608a5bcaa992fc78",
    "FWAVRFService": "0xa084c33Fb7a467307452898b8D58165ebd2E5D9f",
    "FWAToken": "0xa0Df17B5aC76ABaBA36E1450E2cbCd18A620C845",
    "FWATokenHook": "0x2C67ebA8A50AF0dB5Fba55F725247a75CbDA6444",
    "FWAClaim": "0xd4085d38855F17EdF0B1CCBFad7B3846fb305655",
    "FWAWhitelist": "0x854352b275cF6A0DfFCf2983C986FBe9345e17c3",
    "Splitter": "0x1C175b9F0e8C73eD3e677e1cBb1B5A2DD4373Bfe",
    # Not on their deployments page — found 2026-07-28 from a mint in the wild,
    # ~a day old at the time (`nextTokenId` was 29). An ERC-20 → ERC-721 wrapper
    # at whitelisted denominations, with a per-position unwrap timelock and a
    # swappable renderer. Its `fwa()` returns the core pool above, which is the
    # measurement that says what it is FOR: the adapter that lets a fungible
    # token be deposited into their gacha. The verdict on reading it: that
    # mechanic does NOT belong in ours.
    "FWATokenPacks": "0x727C739F07A89f11E883FE0F34937c55e4c3d74A",
    # `renderer()` read off FWATokenPacks the same day. Separate contract on
    # purpose — that split is the one piece of this worth copying outright.
    "FWATokenPacksRenderer": "0x69cC9C633867eee71b17142bbbC2c6AAF14c61a4",
}

BLOCKSCOUT = "https://eth.blockscout.com/api/v2"
ETHERSCAN = "https://api.etherscan.io/v2/api"  # V2; V1 is deprecated and 200s with NOTOK
OPENCHAIN = "https://api.openchain.xyz/signature-database/v1/lookup"
FOURBYTE = "https://www.4byte.directory/api/v1"
TIMEOUT = 25.0

# The key never lives in this file. Etherscan is the ONLY source with broad
# coverage (of the original eight, Blockscout carried four and Sourcify none;
# Token Packs is on neither, re-measured 2026-07-28) — so without a key this
# script degrades to bytecode + a selector index, which is exactly what it did
# on its first run, and what the §1d read was built from. Put the key in
# /data/apps/secrets/keys.env as
# ETHERSCAN_API_KEY and source it, or pass it inline for one run.
ETHERSCAN_KEY = os.getenv("ETHERSCAN_API_KEY", "").strip()


def _json(url: str, tries: int = 3):
    """GET JSON. Returns (data, error) — the error is returned, never swallowed,
    because a reference pull that quietly skips a contract leaves a folder that
    looks complete."""
    last = "unknown"
    for attempt in range(tries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "scry-fwa-reference/1"})
            with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
                return json.loads(r.read().decode("utf-8", "replace")), None
        except urllib.error.HTTPError as e:
            if e.code in (404, 400):
                return None, f"http {e.code}"
            last = f"http {e.code}"
        except Exception as e:  # noqa: BLE001
            last = f"{type(e).__name__}: {e}"
        time.sleep(0.5 * (attempt + 1))
    return None, last


# ------------------------------------------------------------------ sources
def safe_path(root: pathlib.Path, file_path: str, fallback: str) -> pathlib.Path:
    """Verified sources carry their original import paths, including `../` and
    absolute forms. Keep the shape (it is how the imports resolve when reading)
    but never let a fetched string escape the reference folder."""
    raw = (file_path or fallback).replace("\\", "/").lstrip("/")
    parts = [p for p in raw.split("/") if p not in ("", ".", "..")]
    if not parts:
        parts = [fallback]
    p = root.joinpath(*parts)
    if not str(p.resolve()).startswith(str(root.resolve())):
        p = root / fallback
    return p


def write_sources(name: str, addr: str, meta: dict) -> dict:
    root = OUT / name
    files: list[str] = []
    main = meta.get("file_path") or f"{meta.get('name') or name}.sol"
    entries = [(main, meta.get("source_code") or "")]
    for extra in meta.get("additional_sources") or []:
        entries.append((extra.get("file_path") or "", extra.get("source_code") or ""))
    for fp, src in entries:
        if not src:
            continue
        target = safe_path(root, fp, f"{name}.sol")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(src)
        files.append(str(target.relative_to(OUT)))
    # SPDX as it is actually written in the fetched text — the explorer's
    # `license_type` and the file header disagree often enough to check both.
    spdx = sorted({m.group(1) for _, s in entries if s
                   for m in re.finditer(r"SPDX-License-Identifier:\s*([^\s*/]+)", s)})
    return {
        "verified": True,
        "contract_name": meta.get("name"),
        "compiler": meta.get("compiler_version"),
        "evm_version": meta.get("evm_version"),
        "optimizer": meta.get("optimization_enabled"),
        "optimizer_runs": meta.get("optimization_runs"),
        "explorer_license": meta.get("license_type"),
        "spdx_in_source": spdx,
        "constructor_args": meta.get("constructor_args"),
        "files": sorted(files),
        "abi_functions": sorted(
            f.get("name") or "" for f in (meta.get("abi") or []) if f.get("type") == "function"
        ),
        "abi_events": sorted(
            f.get("name") or "" for f in (meta.get("abi") or []) if f.get("type") == "event"
        ),
    }


def etherscan_sources(name: str, addr: str, key: str) -> tuple[dict | None, str | None]:
    """Verified source from Etherscan V2, written out file-for-file.

    Etherscan returns one of three shapes in `SourceCode` and a reader that
    handles only the first gets a single junk file: a bare flattened source, a
    JSON map of path -> {content}, or a standard-json-input wrapped in DOUBLE
    braces (`{{...}}`) — the doubling is Etherscan's own quirk, not JSON.
    """
    q = urllib.parse.urlencode({"chainid": 1, "module": "contract", "action": "getsourcecode",
                                "address": addr, "apikey": key})
    data, err = _json(f"{ETHERSCAN}?{q}")
    if err or not data:
        return None, err or "no answer"
    if str(data.get("status")) != "1" or not data.get("result"):
        # `result` is a human string on failure ("Invalid API Key", "NOTOK").
        return None, f"etherscan: {str(data.get('result'))[:120]}"
    r = data["result"][0]
    raw = (r.get("SourceCode") or "").strip()
    if not raw:
        return None, "etherscan: verified=false (empty SourceCode)"

    entries: list[tuple[str, str]] = []
    settings = None
    if raw.startswith("{{") and raw.endswith("}}"):
        blob = json.loads(raw[1:-1])
        settings = blob.get("settings")
        for path, spec in (blob.get("sources") or {}).items():
            entries.append((path, spec.get("content") or ""))
    elif raw.startswith("{"):
        try:
            blob = json.loads(raw)
            src_map = blob.get("sources") or blob
            settings = blob.get("settings")
            for path, spec in src_map.items():
                entries.append((path, spec.get("content") if isinstance(spec, dict) else str(spec)))
        except json.JSONDecodeError:
            entries.append((f"{name}.sol", raw))
    else:
        entries.append((f"{r.get('ContractName') or name}.sol", raw))

    root = OUT / name
    files: list[str] = []
    for fp, src in entries:
        if not src:
            continue
        target = safe_path(root, fp, f"{name}.sol")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(src)
        files.append(str(target.relative_to(OUT)))
    if settings is not None:
        (root / "settings.json").write_text(json.dumps(settings, indent=2, sort_keys=True) + "\n")
        files.append(f"{name}/settings.json")
    spdx = sorted({m.group(1) for _, s in entries if s
                   for m in re.finditer(r"SPDX-License-Identifier:\s*([^\s*/]+)", s)})
    try:
        abi = json.loads(r.get("ABI") or "[]")
    except json.JSONDecodeError:
        abi = []
    return {
        "verified": True,
        "source": "etherscan v2",
        "contract_name": r.get("ContractName"),
        "compiler": r.get("CompilerVersion"),
        "evm_version": r.get("EVMVersion"),
        "optimizer": r.get("OptimizationUsed") == "1",
        "optimizer_runs": r.get("Runs"),
        "explorer_license": r.get("LicenseType"),
        "spdx_in_source": spdx,
        "proxy": r.get("Proxy"),
        "constructor_args": r.get("ConstructorArguments"),
        "files": sorted(files),
        "abi_functions": sorted(f.get("name") or "" for f in abi if f.get("type") == "function"),
        "abi_events": sorted(f.get("name") or "" for f in abi if f.get("type") == "event"),
    }, None


# ---------------------------------------------------------------- unverified
def selectors_and_topics(code: str) -> tuple[list[str], list[str]]:
    """Pull PUSH4 and PUSH32 constants straight out of the runtime bytecode.

    A solc dispatcher compares `msg.sig` against a PUSH4 per external function,
    and `emit` pushes the event's topic0 as a PUSH32 constant. Scanning for the
    opcodes (0x63, 0x7f) over-collects — any 4-byte literal looks like a
    selector — so the filter is the signature database: a hex string nobody has
    ever registered is dropped rather than guessed at.
    """
    b = bytes.fromhex(code[2:] if code.startswith("0x") else code)
    sels: list[str] = []
    topics: list[str] = []
    i = 0
    n = len(b)
    while i < n:
        op = b[i]
        if op == 0x63 and i + 5 <= n:  # PUSH4
            sels.append("0x" + b[i + 1:i + 5].hex())
            i += 5
            continue
        if op == 0x7F and i + 33 <= n:  # PUSH32
            topics.append("0x" + b[i + 1:i + 33].hex())
            i += 33
            continue
        if 0x60 <= op <= 0x7F:  # any other PUSHn — skip its immediate
            i += 1 + (op - 0x5F)
            continue
        i += 1
    return sorted(set(sels)), sorted(set(topics))


def resolve(kind: str, hexes: list[str]) -> dict[str, list[str]]:
    """Name what we can, keylessly. openchain first (batches, fast), then
    4byte for the leftovers. Unresolved hashes stay in the output as hashes —
    an unnamed selector is a fact about the contract, not a gap to hide."""
    found: dict[str, list[str]] = {}
    for i in range(0, len(hexes), 40):
        chunk = hexes[i:i + 40]
        q = urllib.parse.urlencode({kind: ",".join(chunk)})
        data, err = _json(f"{OPENCHAIN}?{q}")
        if err or not data or not data.get("ok"):
            continue
        table = (data.get("result") or {}).get(kind) or {}
        for h, hits in table.items():
            names = [x.get("name") for x in (hits or []) if x.get("name")]
            if names:
                found[h.lower()] = names
    if kind == "function":
        for h in hexes:
            if h.lower() in found:
                continue
            data, _ = _json(f"{FOURBYTE}/signatures/?hex_signature={h}")
            names = [r.get("text_signature") for r in ((data or {}).get("results") or [])]
            if names:
                found[h.lower()] = names
    return found


def write_surface(name: str, addr: str, code: str, do_sigs: bool) -> dict:
    root = OUT / name
    root.mkdir(parents=True, exist_ok=True)
    (root / f"{name}.runtime.bytecode.txt").write_text(code.strip() + "\n")
    sels, topics = selectors_and_topics(code)
    fn = resolve("function", sels) if do_sigs else {}
    ev = resolve("event", topics) if do_sigs else {}

    lines = [
        f"# {name} — {addr}",
        "",
        "**NOT SOURCE.** This contract is not verified on eth.blockscout.com, is not in",
        "Sourcify, and Etherscan's V2 API needs a key this box does not have. What follows",
        "is reconstructed from the deployed runtime bytecode: 4-byte selectors and 32-byte",
        "event topics resolved against public keyless signature databases (openchain.xyz,",
        "4byte.directory).",
        "",
        "Read it as an INDEX, not a spec. A named selector proves the deployment answers",
        "that signature; it says nothing about what the body does, and an unnamed hash",
        "means nobody registered it, not that it is absent.",
        "",
        f"- runtime bytecode: {len(bytes.fromhex(code[2:] if code.startswith('0x') else code))} bytes",
        f"- keccak256 of the runtime bytecode is in PROVENANCE.json (that is the identity that matters)",
        f"- PUSH4 candidates: {len(sels)} · named: {len(fn)}",
        f"- PUSH32 candidates: {len(topics)} · named events: {len(ev)}",
        "",
        "## Named functions",
        "",
    ]
    for h in sels:
        names = fn.get(h.lower())
        if names:
            lines.append(f"- `{h}` — {' | '.join(sorted(set(names)))}")
    lines += ["", "## Named events", ""]
    for h in topics:
        names = ev.get(h.lower())
        if names:
            lines.append(f"- `{h[:14]}…` — {' | '.join(sorted(set(names)))}")
    unnamed = [h for h in sels if h.lower() not in fn]
    lines += ["", f"## Unresolved 4-byte candidates ({len(unnamed)})", "",
              "Includes real private-to-public selectors AND ordinary 4-byte literals that",
              "are not selectors at all. Listed so the count is honest.", "",
              "  " + " ".join(unnamed)]
    (root / "ABI-SURFACE.md").write_text("\n".join(lines) + "\n")
    return {
        "verified": False,
        "why": "unverified on eth.blockscout.com; absent from Sourcify; Etherscan V2 needs a key",
        "runtime_bytes": len(bytes.fromhex(code[2:] if code.startswith("0x") else code)),
        "selectors_found": len(sels),
        "selectors_named": len(fn),
        "events_named": len(ev),
        "files": [f"{name}/{name}.runtime.bytecode.txt", f"{name}/ABI-SURFACE.md"],
        "named_functions": {h: sorted(set(v)) for h, v in sorted(fn.items())},
        "named_events": {h: sorted(set(v)) for h, v in sorted(ev.items())},
    }


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="fetch the FWA deployment as reference")
    ap.add_argument("--no-sigs", action="store_true", help="skip signature-database lookups")
    ap.add_argument("--no-etherscan", action="store_true",
                    help="ignore ETHERSCAN_API_KEY and use only keyless sources")
    ap.add_argument("--only", help="one contract name from the deployment table")
    a = ap.parse_args(argv)

    OUT.mkdir(parents=True, exist_ok=True)
    prov: dict = {
        "fetched_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "chain": 1,
        "chain_name": "ethereum mainnet",
        "source_of_addresses": "https://www.fwa.fun/docs/deployments",
        "explorer": BLOCKSCOUT,
        "note": ("reference only — not compiled, not imported by src/, no license declared "
                 "upstream. Different randomness, price floor and token on chain 4663: "
                 "most of the pricing does not survive the port."),
        "contracts": {},
    }
    for name, addr in DEPLOYMENT.items():
        if a.only and a.only.lower() != name.lower():
            continue
        meta, err = _json(f"{BLOCKSCOUT}/smart-contracts/{addr}")
        if meta is None:
            prov["contracts"][name] = {"address": addr, "error": err or "no answer"}
            print(f"  {name:14} ERROR {err}")
            continue
        code = meta.get("deployed_bytecode") or ""
        row: dict = {"address": addr}
        if code:
            row["runtime_keccak256"] = "0x" + _keccak(
                bytes.fromhex(code[2:] if code.startswith("0x") else code))
        # Etherscan first when a key is present: it is the only source that has
        # all eight, and it carries the compiler settings the others drop.
        es_row = es_err = None
        if ETHERSCAN_KEY and not a.no_etherscan:
            es_row, es_err = etherscan_sources(name, addr, ETHERSCAN_KEY)
        if es_row:
            row.update(es_row)
            print(f"  {name:14} etherscan  {len(row['files']):>3} files  "
                  f"solc={row['compiler']}  license={row['explorer_license']}  "
                  f"spdx={row['spdx_in_source'] or ['(none)']}")
        elif meta.get("source_code"):
            row.update(write_sources(name, addr, meta))
            row["source"] = "blockscout"
            if es_err:
                row["etherscan_error"] = es_err
            print(f"  {name:14} blockscout {len(row['files']):>3} files  "
                  f"spdx={row['spdx_in_source'] or ['(none)']}")
        elif code:
            if es_err:
                row["etherscan_error"] = es_err
            row.update(write_surface(name, addr, code, not a.no_sigs))
            print(f"  {name:14} bytecode   {row['runtime_bytes']:>6}B  "
                  f"{row['selectors_named']}/{row['selectors_found']} selectors named")
        else:
            row["error"] = "no source and no bytecode returned"
            print(f"  {name:14} EMPTY")
        prov["contracts"][name] = row

    (OUT / "PROVENANCE.json").write_text(json.dumps(prov, indent=2, sort_keys=True) + "\n")
    print(f"\n  {OUT}/PROVENANCE.json written")
    bad = [n for n, r in prov["contracts"].items() if r.get("error")]
    return 1 if bad else 0


def _keccak(raw: bytes) -> str:
    """keccak256, from the meter's own pure-python implementation — the same one
    the claim plans use, so a hash in this folder is reproducible on a box with
    no pysha3 and no web3."""
    try:
        sys.path.insert(0, str(HERE.parent.parent / "meter"))
        from keccak import keccak256  # noqa: PLC0415

        return keccak256(raw).hex()
    except Exception:  # noqa: BLE001
        return "unavailable"


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
