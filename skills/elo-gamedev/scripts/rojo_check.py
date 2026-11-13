#!/usr/bin/env python3
"""rojo_check.py — read a Rojo project and say what the port brief will find
in it, before you build it.

A Roblox game kept with Rojo (a `default.project.json` and its files) comes to
Elo Pros the way a Studio place does: `rojo build` makes a place file,
Fireplace's map editor or `node tools/roblox.mjs place.rbxlx --brief` turns it
into a map and a port brief, and the rules are written onto a Fireplace copy
of its own. This script reads the project as Rojo's default rules do and says,
row by row:

    project   a place or a model, nested projects, the ignore globs
    paths     every $path is there (rojo build stops on a missing one)
    scripts   server, client, module and plugin scripts, and the file each
              becomes in the brief
    remotes   RemoteEvents and RemoteFunctions the project declares; ones a
              script makes at run time the brief will not list
    stores    data stores by name (the brief reads GetDataStore("…") only)
    money     game pass and developer product ids (literal ones only)
    ground    something solid under Workspace: without it the map converter
              stops ("nothing to stand on") and no brief is written
    repo      the port's elo.json and licence, and where it is pushed
    next      the commands, in order, and what the brief should report

It reads and nothing else: no network, no other program, and it writes
nothing. Standard library only.

    python3 rojo_check.py [DIR] [--project FILE] [--port DIR] [--json]

DIR is the Rojo project's folder (default: here). --port is the Fireplace
copy the port lives in, whose elo.json and licence are checked (default: DIR).
Exit 0 when nothing is red, 1 when something would stop `rojo build` or the
brief, 2 when there is no project or it is not JSON.
"""
from __future__ import annotations

import argparse
import fnmatch
import json
import os
import re
import sys
from pathlib import Path, PurePosixPath

# The port brief's own patterns (Fireplace tools/rbx-brief.mjs, read
# 2026-11-04), so a count here is the count the brief will print.
BRIEF_STORE = re.compile(r"""GetDataStore\(\s*["']([^"']+)["']""")
BRIEF_PASS = re.compile(r"(?:UserOwnsGamePassAsync|PlayerHasPass|PromptGamePassPurchase)\([^)]*?(\d{4,})")
BRIEF_PRODUCT = re.compile(r"(?:PromptProductPurchase)\([^)]*?(\d{4,})")
# What the brief misses, said here so a port does not lose it
ORDERED_STORE = re.compile(r"""GetOrderedDataStore\(\s*["']([^"']+)["']""")
ANY_STORE = re.compile(r"Get(?:Ordered)?DataStore\(")
ANY_PASS = re.compile(r"(?:UserOwnsGamePassAsync|PlayerHasPass|PromptGamePassPurchase)\(")
ANY_PRODUCT = re.compile(r"PromptProductPurchase\(")
RECEIPT = re.compile(r"\bProcessReceipt\b")
RUNTIME_REMOTE = re.compile(r"""Instance\.new\(\s*["'](RemoteEvent|RemoteFunction|UnreliableRemoteEvent)["']""")

REMOTES = ("RemoteEvent", "RemoteFunction", "UnreliableRemoteEvent")
# what the map converter stands on (Fireplace tools/rbx-map.mjs PARTS)
SOLID = {"Part", "WedgePart", "CornerWedgePart", "TrussPart", "SpawnLocation", "Seat", "VehicleSeat",
         "SkateboardPlatform", "MeshPart", "UnionOperation", "IntersectOperation", "PartOperation",
         "FlagStand", "Platform"}
# the brief's file for each script class (rbx-brief.mjs SCRIPT_EXT)
BRIEF_EXT = {"Script": ".server.luau", "LocalScript": ".client.luau", "ModuleScript": ".luau"}
# the licence files the store's checklist reads (meter/repolane.py
# LICENCE_FILES), built as it builds them rather than spelled out
LICENCES = tuple(f"{b}{ext}" for b in ("LICENSE", "LICENCE") for ext in ("", ".md", ".txt")) + ("COPYING",)
TEMPLATE_SLUG = "fireplace"   # the template's own slug: a copy must name its own game


class Unreadable(Exception):
    """No project here, or one that is not JSON."""


# ── reading Rojo's files ─────────────────────────────────────────────────────
def _strip_jsonc(text: str) -> str:
    """JSON with comments and trailing commas (Rojo's .jsonc) as plain JSON."""
    out, i, n, in_str = [], 0, len(text), False
    while i < n:
        c = text[i]
        if in_str:
            out.append(c)
            if c == "\\" and i + 1 < n:
                out.append(text[i + 1])
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            out.append(c)
        elif text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        else:
            out.append(c)
        i += 1
    return re.sub(r",(\s*[}\]])", r"\1", "".join(out))


def read_json(path: Path):
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as e:
        raise Unreadable(f"{path.name} could not be read: {e}") from None
    try:
        return json.loads(_strip_jsonc(text) if path.suffix == ".jsonc" else text)
    except ValueError as e:
        raise Unreadable(f"{path.name} is not JSON: {e}") from None


def find_project(root: Path, given: str | None) -> Path:
    if given:
        p = Path(given)
        p = p if p.is_absolute() else root / p
        if not p.is_file():
            raise Unreadable(f"there is no {given}")
        return p
    for name in ("default.project.json", "default.project.jsonc"):
        if (root / name).is_file():
            return root / name
    found = sorted(p for p in root.iterdir() if p.is_file()
                   and (p.name.endswith(".project.json") or p.name.endswith(".project.jsonc"))) \
        if root.is_dir() else []
    if len(found) == 1:
        return found[0]
    if found:
        raise Unreadable("more than one *.project.json here: name one with --project")
    raise Unreadable(f"no default.project.json in {root}: is this a Rojo project? (`rojo init` makes one)")


def _split(name: str) -> tuple[str, str]:
    """(stem, kind) of a file by Rojo's default rules."""
    low = name.lower()
    for suf, kind in ((".server.luau", "server"), (".server.lua", "server"),
                      (".client.luau", "client"), (".client.lua", "client"),
                      (".plugin.luau", "plugin"), (".plugin.lua", "plugin"),
                      (".model.jsonc", "model"), (".model.json", "model"),
                      (".meta.jsonc", "meta"), (".meta.json", "meta"),
                      (".project.jsonc", "project"), (".project.json", "project"),
                      (".luau", "module"), (".lua", "module"),
                      (".rbxmx", "rbxm"), (".rbxm", "rbxm"),
                      (".jsonc", "data"), (".json", "data"), (".toml", "data"),
                      (".yaml", "data"), (".yml", "data"),
                      (".txt", "text"), (".csv", "csv")):
        if low.endswith(suf):
            return name[: len(name) - len(suf)], kind
    return name, "other"


class Project:
    """One read of a Rojo project: the instances it makes, as Rojo would."""

    def __init__(self, path: Path):
        self.path = path
        self.root = path.parent
        self.doc = read_json(path)
        if not isinstance(self.doc, dict) or not isinstance(self.doc.get("tree"), dict):
            raise Unreadable(f"{path.name} has no `tree`: it is not a Rojo project file")
        self.name = str(self.doc.get("name") or path.name.split(".")[0])
        self.place = self.doc["tree"].get("$className") == "DataModel"
        self.legacy = self.doc.get("emitLegacyScripts", True) is not False
        self.ignore = [str(g) for g in self.doc.get("globIgnorePaths") or [] if isinstance(g, str)]
        self.sync_rules = bool(self.doc.get("syncRules"))
        self.scripts: list[dict] = []      # {class, kind, path, file, source}
        self.remotes: list[dict] = []      # {class, path, file}
        self.solids: list[str] = []        # instance paths of solid parts under Workspace
        self.opaque: list[str] = []        # .rbxm/.rbxmx models, which this can't see inside
        self.missing: list[str] = []       # $paths that are not there
        self.nested: list[str] = []
        self.loops: list[str] = []
        self.terrain = False
        self._seen = {path.resolve()}
        top = [] if self.place else [self.name]
        self._node(self.doc["tree"], top, None, self.root)

    # an instance path as the brief writes it: the service first
    @staticmethod
    def _dotted(parts: list[str]) -> str:
        return ".".join(parts)

    def _ignored(self, f: Path) -> bool:
        try:
            rel = PurePosixPath(f.resolve().relative_to(self.root.resolve()).as_posix())
        except ValueError:
            return False
        return any(fnmatch.fnmatch(str(rel), g) or rel.match(g) for g in self.ignore)

    def _note(self, cls: str | None, parts: list[str], file: str | None) -> None:
        if not cls:
            return
        under_ws = bool(parts) and parts[0] == "Workspace"
        if cls in REMOTES:
            self.remotes.append({"class": cls, "path": self._dotted(parts), "file": file})
        if cls in SOLID and under_ws:
            self.solids.append(self._dotted(parts))
        if cls == "Terrain":
            self.terrain = True

    def _node(self, node: dict, parts: list[str], cls: str | None, base: Path) -> None:
        cls = node.get("$className") or cls
        if not cls and len(parts) == 1 and self.place:
            cls = parts[0]          # a service, named for itself
        self._note(cls, parts, None)
        if "$path" in node:
            self._path(base / str(node["$path"]), parts, cls, str(node["$path"]))
        for k, v in node.items():
            if not k.startswith("$") and isinstance(v, dict):
                self._node(v, parts + [k], None, base)

    def _path(self, p: Path, parts: list[str], cls: str | None, shown: str) -> None:
        if not p.exists():
            self.missing.append(shown)
            return
        if p.is_file():
            self._file(p, parts, named=True)
        else:
            self._dir(p, parts)

    def _dir(self, d: Path, parts: list[str]) -> None:
        init = None
        for name in ("init.server.luau", "init.server.lua", "init.client.luau", "init.client.lua",
                     "init.plugin.luau", "init.plugin.lua", "init.luau", "init.lua"):
            if (d / name).is_file() and not self._ignored(d / name):
                init = d / name
                break
        meta = next((d / n for n in ("init.meta.json", "init.meta.jsonc") if (d / n).is_file()), None)
        if init is not None:
            self._file(init, parts, named=True)
        elif meta is not None:
            try:
                m = read_json(meta)
                self._note(m.get("className") if isinstance(m, dict) else None, parts, None)
            except Unreadable:
                pass
        for f in sorted(d.iterdir(), key=lambda x: x.name):
            if self._ignored(f) or f.name.startswith(".") or f == init or f == meta:
                continue
            if f.is_dir():
                self._dir(f, parts + [f.name])
            else:
                self._file(f, parts, named=False)

    def _file(self, f: Path, parts: list[str], named: bool) -> None:
        stem, kind = _split(f.name)
        here = parts if named or stem == "init" else parts + [stem]
        rel = os.path.relpath(f, self.root)
        if kind in ("server", "client", "plugin", "module"):
            if kind == "client":
                klass = "LocalScript" if self.legacy else "Script"
            else:
                klass = {"server": "Script", "plugin": "Script", "module": "ModuleScript"}[kind]
            try:
                src = f.read_text(encoding="utf-8", errors="replace")
            except OSError:
                src = ""
            self.scripts.append({"class": klass, "kind": kind, "path": self._dotted(here), "file": rel,
                                 "source": src})
        elif kind == "model":
            try:
                m = read_json(f)
            except Unreadable:
                return
            self._model(m, here, rel)
        elif kind == "project":
            r = f.resolve()
            if r in self._seen:
                self.loops.append(rel)
                return
            self._seen.add(r)
            try:
                sub = read_json(f)
            except Unreadable:
                return
            if isinstance(sub, dict) and isinstance(sub.get("tree"), dict):
                self.nested.append(rel)
                self._node(sub["tree"], here, None, f.parent)
        elif kind == "rbxm":
            self.opaque.append(rel)

    def _model(self, m, parts: list[str], rel: str) -> None:
        if not isinstance(m, dict):
            return
        self._note(m.get("className") or m.get("ClassName"), parts, rel)
        for ch in m.get("children") or m.get("Children") or []:
            if isinstance(ch, dict):
                self._model(ch, parts + [str(ch.get("name") or ch.get("Name") or "_")], rel)


# ── what the brief will make of it ───────────────────────────────────────────
def brief_names(scripts: list[dict]) -> list[str]:
    """The file each script becomes in the brief (rbx-brief.mjs: the path's
    parts with anything but word characters, spaces and dashes made `_`, a
    second one of the same name ` 2`)."""
    taken, out = set(), []
    for s in scripts:
        ext = BRIEF_EXT[s["class"]]
        p = "scripts/" + "/".join((re.sub(r"[^\w -]+", "_", part, flags=re.ASCII).strip() or "_")
                                   for part in s["path"].split("."))
        f, k = p + ext, 2
        while f in taken:
            f = f"{p} {k}{ext}"
            k += 1
        taken.add(f)
        out.append(f)
    return out


def _origin(root: Path) -> str | None:
    """The `origin` remote's url, read from .git/config as text."""
    for d in [root, *root.parents]:
        cfg = d / ".git" / "config"
        if cfg.is_file():
            try:
                text = cfg.read_text(encoding="utf-8", errors="replace")
            except OSError:
                return None
            m = re.search(r'\[remote "origin"\][^\[]*?url\s*=\s*(\S+)', text)
            return m.group(1) if m else None
    return None


def row(key: str, ok, label: str, detail: str = "", fix: str = "") -> dict:
    return {"key": key, "ok": ok, "label": label, "detail": detail, "fix": fix}


def check(root: Path, project: str | None = None, port: Path | None = None) -> dict:
    root = root.resolve()
    proj = Project(find_project(root, project))
    port = (port or root).resolve()
    rows = []

    bits = ["a place" if proj.place else "a model (build it with -o place.rbxmx)"]
    if proj.nested:
        bits.append(f"{len(proj.nested)} nested project(s)")
    if proj.ignore:
        bits.append(f"ignores {', '.join(proj.ignore)}")
    amber = proj.sync_rules or bool(proj.loops)
    rows.append(row("project", None if amber else True, f"{proj.name}: " + ", ".join(bits),
                    ("its own syncRules change how files map; the counts below follow Rojo's defaults"
                     if proj.sync_rules else "") + (f" a project includes itself: {', '.join(proj.loops)}"
                                                    if proj.loops else "")))

    rows.append(row("paths", not proj.missing, "every $path is there" if not proj.missing
                    else f"{len(proj.missing)} $path(s) missing: {', '.join(proj.missing)}",
                    fix="" if not proj.missing else "make the folder or fix the path: rojo build stops on it"))

    by = {k: sum(1 for s in proj.scripts if s["kind"] == k) for k in ("server", "client", "module", "plugin")}
    names = brief_names(proj.scripts)
    lines = sum(s["source"].count("\n") + 1 for s in proj.scripts)
    relabel = by["plugin"] or (not proj.legacy and by["client"])
    rows.append(row("scripts", None if relabel else True,
                    f"{len(proj.scripts)} scripts, {lines} lines: {by['server']} server, {by['client']} client, "
                    f"{by['module']} module" + (f", {by['plugin']} plugin" if by["plugin"] else ""),
                    ("the brief names a plugin script, and a client script under emitLegacyScripts: false, "
                     ".server.luau; read those by where they sat") if relabel else ""))

    runtime = sorted({f"{m.group(1)} in {s['file']}" for s in proj.scripts
                      for m in RUNTIME_REMOTE.finditer(s["source"])})
    rows.append(row("remotes", None if runtime else True,
                    f"{len(proj.remotes)} remote(s) declared" + (": " + ", ".join(r["path"] for r in proj.remotes[:8])
                                                                 if proj.remotes else ""),
                    ("made at run time, so the brief will not list them: " + "; ".join(runtime)) if runtime else ""))

    stores, ordered, loose = set(), set(), 0
    for s in proj.scripts:
        src = s["source"]
        found = BRIEF_STORE.findall(src)
        stores.update(found)
        ordered.update(ORDERED_STORE.findall(src))
        loose += len(ANY_STORE.findall(src)) - len(found) - len(ORDERED_STORE.findall(src))
    extra = []
    if ordered:
        extra.append(f"ordered (the brief misses these): {', '.join(sorted(ordered))}")
    if loose > 0:
        extra.append(f"{loose} named by a variable, which neither the brief nor this can read")
    rows.append(row("stores", None if extra else True,
                    f"{len(stores)} data store(s)" + (f": {', '.join(sorted(stores))}" if stores else ""),
                    "; ".join(extra), "on Elo Pros these are cloud saves (and boards, for an ordered store)"
                    if stores or ordered else ""))

    passes, products, loose_money, receipt = set(), set(), 0, False
    for s in proj.scripts:
        src = s["source"]
        p_found, d_found = BRIEF_PASS.findall(src), BRIEF_PRODUCT.findall(src)
        passes.update(p_found)
        products.update(d_found)
        loose_money += len(ANY_PASS.findall(src)) - len(p_found) + len(ANY_PRODUCT.findall(src)) - len(d_found)
        receipt = receipt or bool(RECEIPT.search(src))
    note = []
    if loose_money > 0:
        note.append(f"{loose_money} call(s) with no literal id, which the brief won't list")
    if receipt:
        note.append("a ProcessReceipt handler: its products are the ones it grants")
    rows.append(row("money", None if note else True,
                    f"{len(passes)} game pass(es), {len(products)} developer product(s)", "; ".join(note),
                    "on Elo Pros, passes and products in elo.json's economy block" if passes or products or receipt
                    else ""))

    if proj.solids:
        rows.append(row("ground", True, f"{len(proj.solids)} solid part(s) under Workspace, e.g. {proj.solids[0]}"))
    elif proj.opaque or proj.terrain:
        rows.append(row("ground", None, "nothing solid this can see under Workspace",
                        "a .rbxm model or Terrain may hold the ground; this can't see inside them",
                        "if the brief stops with \"nothing to stand on\", add a Baseplate Part as rojo init does"))
    else:
        rows.append(row("ground", False, "nothing solid under Workspace",
                        "the map converter stops with \"nothing to stand on\" and writes no brief",
                        "add a Baseplate Part under Workspace in the project, as `rojo init` does, or save the "
                        "place from Studio with its map"))

    elo = port / "elo.json"
    repo_bits, fixes, template, missing = [], [], False, False
    if elo.is_file():
        try:
            doc = read_json(elo)
            game = str(doc.get("game") or "") if isinstance(doc, dict) else ""
        except Unreadable:
            game = ""
        template = game == TEMPLATE_SLUG
        repo_bits.append('elo.json still names the template ("fireplace")' if template
                         else f"elo.json names {game or 'no game yet'}")
        if template:
            fixes.append("give elo.json your own game's slug")
        elif game and isinstance(doc, dict):
            # what a copy of the template carries over that is not its own
            jobs = [str(j.get("id") or "") for j in doc.get("jobs") or [] if isinstance(j, dict)]
            stray = [j for j in jobs if not j.startswith(game + "-")]
            if stray:
                template = True
                repo_bits.append(f"{len(stray)} job id(s) not under {game}-: {', '.join(stray[:3])}")
                fixes.append(f"rename each job's id to start with {game}-")
            web = doc.get("web") if isinstance(doc.get("web"), dict) else {}
            if f"/{TEMPLATE_SLUG}/releases/".lower() in str(web.get("url") or "").lower():
                template = True
                repo_bits.append("the web block is the template's own build")
                fixes.append("cut your own release and draft its url and sha256, or take the web block out")
    else:
        missing = True
        repo_bits.append("no elo.json yet")
        fixes.append("dev(what='draft', repo=…) writes it")
    lic = next((n for n in LICENCES if (port / n).is_file()), None)
    repo_bits.append(f"licence {lic}" if lic else "no licence file")
    if not lic:
        missing = True
        fixes.append("a LICENSE at the root: a sale waits on one")
    origin = _origin(port)
    if origin and "github.com" in origin:
        repo_bits.append(f"pushed to {origin}")
    else:
        missing = True
        repo_bits.append("not pushed to GitHub yet")
        fixes.append("push it to a public GitHub repo: the store reads the default branch")
    rows.append(row("repo", False if template else (None if missing else True), "; ".join(repo_bits),
                    "a listed repo is public, its scripts too", "; ".join(fixes)))

    out_name = "place.rbxlx" if proj.place else "place.rbxmx"
    project_arg = os.path.relpath(proj.path, root)
    rows.append(row("next", None, "then", "\n".join([
        f"rojo build {project_arg} -o {out_name}",
        f"node tools/roblox.mjs {out_name} --brief      (in your Fireplace copy, or the map editor's "
        "'download the port brief')",
        f"the brief should say {len(proj.scripts)} scripts, {len(proj.remotes)} remotes, "
        f"{len(stores)} data stores",
        "dev(what='check', repo='https://github.com/you/your-port')"])))

    return {"project": str(proj.path), "name": proj.name, "place": proj.place, "rows": rows,
            "scripts": [{"class": s["class"], "kind": s["kind"], "path": s["path"], "file": s["file"],
                         "brief": b} for s, b in zip(proj.scripts, names)],
            "remotes": proj.remotes, "stores": sorted(stores), "ordered_stores": sorted(ordered),
            "passes": sorted(passes), "products": sorted(products),
            "ok": not any(r["ok"] is False for r in rows)}


def show(result: dict) -> str:
    mark = {True: "ok  ", False: "FAIL", None: "--  "}
    out = []
    for r in result["rows"]:
        if r["key"] == "next":
            out.append("\nnext:")
            out.extend("  " + ln for ln in r["detail"].split("\n"))
            continue
        out.append(f"{mark[r['ok']]}  {r['key']:<8} {r['label']}")
        if r["detail"]:
            out.append(f"              {r['detail']}")
        if r["fix"] and r["ok"] is not True:
            out.append(f"              → {r['fix']}")
    return "\n".join(out)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="rojo_check.py", description=__doc__.split("\n\n")[0])
    ap.add_argument("dir", nargs="?", default=".", help="the Rojo project's folder")
    ap.add_argument("--project", help="the project file, when it isn't default.project.json")
    ap.add_argument("--port", help="the Fireplace copy the port lives in (its elo.json and licence)")
    ap.add_argument("--json", action="store_true", help="the rows as JSON")
    a = ap.parse_args(argv)
    try:
        res = check(Path(a.dir), a.project, Path(a.port) if a.port else None)
    except Unreadable as e:
        print(f"rojo_check: {e}", file=sys.stderr)
        return 2
    print(json.dumps(res, indent=1) if a.json else show(res))
    return 0 if res["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
