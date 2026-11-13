#!/usr/bin/env python3
"""Every file and folder Unity imports from this package has a .meta beside it.

    python3 sdk/unity/Tests~/unity_meta.py           # check
    python3 sdk/unity/Tests~/unity_meta.py --write   # make the missing ones

A package installed from git is read-only to Unity, so it cannot write the
.meta files itself: an asset without one is ignored, with a console warning
the developer may never read. The GUIDs are derived from the path, so a
re-run makes the same file and a moved file gets a new one. Folders ending in
`~` (Tests~, Samples~) are invisible to Unity and get none.
"""
from __future__ import annotations

import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

COMMON = "  externalObjects: {}\n  userData: \n  assetBundleName: \n  assetBundleVariant: \n"

PLUGIN_WEBGL_ONLY = """PluginImporter:
  externalObjects: {}
  serializedVersion: 2
  iconMap: {}
  executionOrder: {}
  defineConstraints: []
  isPreloaded: 0
  isOverridable: 0
  isExplicitlyReferenced: 0
  validateReferences: 1
  platformData:
  - first:
      Any:
    second:
      enabled: 0
      settings: {}
  - first:
      Editor: Editor
    second:
      enabled: 0
      settings:
        DefaultValueInitialized: true
  - first:
      WebGL: WebGL
    second:
      enabled: 1
      settings: {}
  userData:
  assetBundleName:
  assetBundleVariant:
"""

MONO = """MonoImporter:
  externalObjects: {}
  serializedVersion: 2
  defaultReferences: []
  executionOrder: 0
  icon: {instanceID: 0}
  userData:
  assetBundleName:
  assetBundleVariant:
"""


def guid(rel: str) -> str:
    return hashlib.md5(("com.elopros.sdk/" + rel).encode()).hexdigest()


def body(path: Path) -> str:
    if path.is_dir():
        return "folderAsset: yes\nDefaultImporter:\n" + COMMON
    name = path.name
    if name == "package.json":
        return "PackageManifestImporter:\n" + COMMON
    if path.suffix == ".cs":
        return MONO
    if path.suffix == ".asmdef":
        return "AssemblyDefinitionImporter:\n" + COMMON
    if path.suffix == ".jslib":
        return PLUGIN_WEBGL_ONLY
    if path.suffix in (".md", ".json", ".txt"):
        return "TextScriptImporter:\n" + COMMON
    return "DefaultImporter:\n" + COMMON


def assets() -> list[Path]:
    out = []
    for p in sorted(ROOT.rglob("*")):
        rel = p.relative_to(ROOT)
        if any(part.endswith("~") or part.startswith(".") for part in rel.parts):
            continue
        if p.suffix == ".meta":
            continue
        out.append(p)
    return out


def main() -> int:
    write = "--write" in sys.argv
    bad, guids = [], {}
    for p in assets():
        rel = p.relative_to(ROOT).as_posix()
        want = f"fileFormatVersion: 2\nguid: {guid(rel)}\n" + body(p)
        meta = p.with_name(p.name + ".meta")
        if meta.exists() and meta.read_text() == want:
            pass
        elif write:
            meta.write_text(want)
            print(f"wrote {meta.relative_to(ROOT)}")
        else:
            bad.append(f"{rel}: {'stale' if meta.exists() else 'missing'} .meta")
        guids.setdefault(guid(rel), []).append(rel)
    for p in sorted(ROOT.rglob("*.meta")):
        rel = p.relative_to(ROOT)
        if any(part.endswith("~") for part in rel.parts):
            continue
        if not p.with_name(p.name[: -len(".meta")]).exists():
            bad.append(f"{rel.as_posix()}: a .meta with no asset")
    bad += [f"guid {g} is shared by {v}" for g, v in guids.items() if len(v) > 1]
    for b in bad:
        print(f"  FAIL  {b}")
    print(f"{len(guids)} assets, {len(bad)} problems")
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
