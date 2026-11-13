#!/usr/bin/env python3
"""test_rojo_check.py — rojo_check.py against Rojo projects built here, offline.

Every fixture is written to a temp folder by this file (no Luau is committed),
shaped as `rojo init` makes a place, with what a Roblox game leans on added.
The brief names and counts are the ones Fireplace's tools/rbx-brief.mjs
printed for the same project in the 2026-11-04 run.

    python3 test_rojo_check.py
"""
from __future__ import annotations

import io
import json
import re
import sys
import tempfile
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import rojo_check as R  # noqa: E402

passed = failed = 0


def ok(cond, what, detail=""):
    global passed, failed
    if cond:
        passed += 1
        print(f"  ok: {what}")
    else:
        failed += 1
        print(f"  FAIL: {what}" + (f" — {detail}" if detail else ""))


def write(root: Path, files: dict) -> Path:
    for rel, text in files.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text if isinstance(text, str) else json.dumps(text, indent=1), encoding="utf-8")
    return root


SERVER = '''local DataStoreService = game:GetService("DataStoreService")
local MarketplaceService = game:GetService("MarketplaceService")
local saves = DataStoreService:GetDataStore("PlayerData")
local wins = DataStoreService:GetOrderedDataStore("Wins")
local other = DataStoreService:GetDataStore(STORE_NAME)
local flare = Instance.new("RemoteEvent")
game.Players.PlayerAdded:Connect(function(p)
  if MarketplaceService:UserOwnsGamePassAsync(p.UserId, 1234567) then end
  if MarketplaceService:UserOwnsGamePassAsync(p.UserId, VIP) then end
end)
MarketplaceService.ProcessReceipt = function(info) end
'''
CLIENT = 'game:GetService("MarketplaceService"):PromptProductPurchase(game.Players.LocalPlayer, 7654321)\n'


def place(root: Path, **over) -> Path:
    tree = {
        "$className": "DataModel",
        "ReplicatedStorage": {"Shared": {"$path": "src/shared"},
                              "Remotes": {"$className": "Folder", "Hit": {"$className": "RemoteEvent"},
                                          "GetScore": {"$className": "RemoteFunction"}}},
        "ServerScriptService": {"Server": {"$path": "src/server"}},
        "StarterPlayer": {"StarterPlayerScripts": {"Client": {"$path": "src/client"}}},
        "Workspace": {"Baseplate": {"$className": "Part", "$properties": {"Size": [512, 20, 512]}}},
    }
    doc = {"name": "game", "tree": tree, **over.pop("doc", {})}
    files = {
        "default.project.json": doc,
        "src/server/init.server.luau": SERVER,
        "src/client/init.client.luau": CLIENT,
        "src/shared/Hello.luau": "return {}\n",
        "src/shared/Old.lua": "return 1\n",
        "src/shared/Ping.model.json": {"className": "UnreliableRemoteEvent"},
        "src/shared/Effects/init.meta.json": {"className": "Folder"},
        "src/shared/Effects/Spark.luau": "return 2\n",
    }
    files.update(over.pop("files", {}))
    return write(root, files)


def run(args) -> tuple[int, str, str]:
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = R.main([str(a) for a in args])
    return code, out.getvalue(), err.getvalue()


def rows(res) -> dict:
    return {r["key"]: r for r in res["rows"]}


T = Path(tempfile.mkdtemp(prefix="rojo-check-test-"))

print("[a place as rojo init makes it, with a game in it]")
g = place(T / "game")
before = sorted((p.relative_to(g).as_posix(), p.stat().st_mtime_ns) for p in g.rglob("*"))
res = R.check(g)
r = rows(res)
ok(res["place"] is True and r["project"]["ok"] is True, "a DataModel tree is a place")
ok(r["paths"]["ok"] is True, "every $path is there")
kinds = {k: sum(1 for s in res["scripts"] if s["kind"] == k) for k in ("server", "client", "module")}
ok(kinds == {"server": 1, "client": 1, "module": 3}, "scripts by Rojo's suffixes: .server, .client, .luau and .lua",
   str(kinds))
ok({s["path"] for s in res["scripts"]} >= {"ServerScriptService.Server", "StarterPlayer.StarterPlayerScripts.Client",
                                           "ReplicatedStorage.Shared.Hello", "ReplicatedStorage.Shared.Effects.Spark"},
   "an init script is its folder; a folder's files sit under it")
briefs = {s["path"]: s["brief"] for s in res["scripts"]}
ok(briefs["ServerScriptService.Server"] == "scripts/ServerScriptService/Server.server.luau"
   and briefs["StarterPlayer.StarterPlayerScripts.Client"]
   == "scripts/StarterPlayer/StarterPlayerScripts/Client.client.luau"
   and briefs["ReplicatedStorage.Shared.Hello"] == "scripts/ReplicatedStorage/Shared/Hello.luau",
   "each script's brief file, as the 2026-11-04 brief wrote them", str(briefs))
ok({x["class"] for x in res["remotes"]} == {"RemoteEvent", "RemoteFunction", "UnreliableRemoteEvent"},
   "remotes from $className and from a .model.json", str(res["remotes"]))
ok(r["remotes"]["ok"] is None and "Instance.new" not in r["remotes"]["detail"]
   and "RemoteEvent in src/server" in r["remotes"]["detail"], "a remote made at run time is flagged: the brief misses it")
ok(res["stores"] == ["PlayerData"] and res["ordered_stores"] == ["Wins"],
   "stores by the brief's pattern, ordered ones apart")
ok("ordered" in r["stores"]["detail"] and "1 named by a variable" in r["stores"]["detail"],
   "…the ordered store and the one named by a variable are said", r["stores"]["detail"])
ok(res["passes"] == ["1234567"] and res["products"] == ["7654321"], "pass and product ids by the brief's patterns")
ok("1 call(s) with no literal id" in r["money"]["detail"] and "ProcessReceipt" in r["money"]["detail"],
   "…a pass named by a variable and a ProcessReceipt handler are said", r["money"]["detail"])
ok(r["ground"]["ok"] is True and "Workspace.Baseplate" in r["ground"]["label"], "the Baseplate is ground")
nxt = r["next"]["detail"]
ok("rojo build default.project.json -o place.rbxlx" in nxt and "tools/roblox.mjs place.rbxlx --brief" in nxt
   and f"{len(res['scripts'])} scripts, 3 remotes, 1 data stores" in nxt, "the next commands, and what the brief should say")
ok(res["ok"] is True, "nothing red: exit 0")
code, out, _ = run([g])
ok(code == 0 and "ok    ground" in out and "next:" in out, "the text output", out[:200])
code, out, _ = run([g, "--json"])
ok(code == 0 and json.loads(out)["scripts"] == res["scripts"], "--json is the same result")
after = sorted((p.relative_to(g).as_posix(), p.stat().st_mtime_ns) for p in g.rglob("*"))
ok(before == after, "a run writes nothing")

print("[names the brief has to bend]")
c = place(T / "collide", files={"src/shared/A b!.luau": "return 1\n", "src/shared/A b_.luau": "return 2\n"})
names = [s["brief"] for s in R.check(c)["scripts"] if "A b" in s["path"]]
ok(set(names) == {"scripts/ReplicatedStorage/Shared/A b_.luau", "scripts/ReplicatedStorage/Shared/A b_ 2.luau"},
   "odd characters become _, and a second of the same name is ' 2'", str(names))

print("[what stops the build or the brief]")
m = place(T / "missing", doc={"tree": {"$className": "DataModel", "ServerScriptService": {"S": {"$path": "src/nope"}},
                                       "Workspace": {"Base": {"$className": "Part"}}}})
res = R.check(m)
ok(rows(res)["paths"]["ok"] is False and "src/nope" in rows(res)["paths"]["label"] and run([m])[0] == 1,
   "a missing $path is red, and exit 1")
n = place(T / "noground", doc={"tree": {"$className": "DataModel", "ServerScriptService": {"S": {"$path": "src/server"}}}})
res = R.check(n)
ok(rows(res)["ground"]["ok"] is False and "nothing to stand on" in rows(res)["ground"]["detail"]
   and "Baseplate" in rows(res)["ground"]["fix"], "no ground is red, with the converter's own words and the fix")
o = place(T / "opaque", doc={"tree": {"$className": "DataModel", "Workspace": {"Map": {"$path": "map.rbxm"}}}},
          files={"map.rbxm": "binary"})
ok(rows(R.check(o))["ground"]["ok"] is None, "a .rbxm map can't be seen inside: not red, said")
code, _, err = run([T / "nowhere"])
ok(code == 2 and "no default.project.json" in err, "no project: exit 2, said", err)
bad = write(T / "badjson", {"default.project.json": "{not json"})
ok(run([bad])[0] == 2, "a project that isn't JSON: exit 2")

print("[the rest of Rojo's project file]")
j = write(T / "jsonc", {"default.project.jsonc": '{\n // a comment\n "name": "j", "tree": {"$className": "DataModel",'
                        ' "Workspace": {"B": {"$className": "Part"}}, /* block */ },\n}\n'})
ok(R.check(j)["name"] == "j", "a .jsonc project, comments and trailing commas")
model = write(T / "model", {"default.project.json": {"name": "lib", "tree": {"$path": "src"}}, "src/init.luau": "return {}"})
res = R.check(model)
ok(res["place"] is False and "rbxmx" in rows(res)["next"]["detail"], "a model project builds to .rbxmx")
nest = place(T / "nest", files={"src/shared/Lib.project.json": {"name": "Lib", "tree": {"$path": "lib"}},
                                "src/shared/lib/init.luau": "return {}"})
res = R.check(nest)
ok(any(s["path"] == "ReplicatedStorage.Shared.Lib" for s in res["scripts"]) and "nested" in rows(res)["project"]["label"],
   "a nested project is read where it sits")
ign = place(T / "ignore", doc={"globIgnorePaths": ["src/shared/Old.lua"]})
ok(not any(s["path"].endswith(".Old") for s in R.check(ign)["scripts"]), "globIgnorePaths leaves a file out")
leg = place(T / "legacy", doc={"emitLegacyScripts": False})
res = R.check(leg)
ok(rows(res)["scripts"]["ok"] is None and any(s["kind"] == "client" and s["class"] == "Script" for s in res["scripts"]),
   "emitLegacyScripts false: a client script is a Script, and the brief's label is said")

print("[the port's repo]")
port = write(T / "port", {"elo.json": {"game": "fireplace"}, "LICENSE": "CC0"})
ok(rows(R.check(g, port=port))["repo"]["ok"] is False, "elo.json still naming the template is red")
port2 = write(T / "port2", {"elo.json": {"game": "my-port", "jobs": [{"id": "fireplace-theme"}],
                                         "web": {"url": "https://github.com/AnthonE/Fireplace/releases/download/x/site.zip"}},
                            "LICENCE" + ".md": "MIT"})
rr = rows(R.check(g, port=port2))["repo"]
ok(rr["ok"] is False and "fireplace-theme" in rr["label"] and "template's own build" in rr["label"],
   "the template's job ids and web build, carried over, are red", rr["label"])
port3 = write(T / "port3", {"elo.json": {"game": "my-port"}, "LICENCE" + ".md": "MIT",
                            ".git/config": '[remote "origin"]\n\turl = https://github.com/you/my-port.git\n'})
rr = rows(R.check(g, port=port3))["repo"]
ok(rr["ok"] is True and "LICENCE" + ".md" in rr["label"] and "github.com/you/my-port" in rr["label"],
   "a licence by any of the checklist's names, and the origin read from .git/config")
ok(rows(R.check(g))["repo"]["ok"] is None, "no elo.json, licence or push yet: said, not red")

print("[it only reads]")
src = (HERE / "rojo_check.py").read_text(encoding="utf-8")
mods = set(re.findall(r"^\s*(?:import|from)\s+([a-zA-Z_.]+)", src, re.M))
ok(mods <= {"__future__", "argparse", "fnmatch", "json", "os", "re", "sys", "pathlib"},
   "the standard library only, and nothing that reaches the network or runs a program", str(mods))
ok("rbx-brief.mjs" in src and R.BRIEF_STORE.pattern == r"""GetDataStore\(\s*["']([^"']+)["']""",
   "the brief's patterns are pinned, with where they come from")

print(f"\n{'all good' if not failed else 'FAILED'} — {passed} checks passed" + (f", {failed} failed" if failed else ""))
sys.exit(1 if failed else 0)
