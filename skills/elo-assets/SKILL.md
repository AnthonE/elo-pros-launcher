---
name: elo-assets
description: Use when an AI agent (Claude Code, Codex, Cursor, Hermes, OpenClaw, any harness with a shell) should find and add game assets from the Elo Pros asset store to a project — models, sprites, sounds, shaders, plugins, each 99¢ or less, with the licence and credit line written for you — or make an asset (for example a model in Blender through a Blender MCP server), put it in a GitHub release, write its elo-asset.json and list it. No wallet is needed to list; Elo never holds a key.
license: MIT
metadata:
  version: "1.2.0"
  hermes:
    category: development
    tags: [games, assets, blender, gamedev, modding, github]
  openclaw:
    category: development
    tags: [games, assets, blender, gamedev, modding, github]
---

# elo-assets — use and make assets for games

The asset store at https://elopros.com/assets.html sells files for making
games: models, textures, sprites, sounds, music, shaders, UI kits, engine
plugins. Each one costs 0¢ to 99¢. An asset is either uploaded with one
wallet signature (its files pinned to IPFS) or published from a public GitHub
repo (its files are that repo's release assets, described by `elo-asset.json`
at its root). Every file carries a sha256.

Our own games' art is on the store too: `assets(what='shelf')` lists it (the
house's shelf, `elopros`), and `assets(tag='gates')` finds Gates' models —
Triple T, the fel animals and the weapon skins, free, CC-BY-4.0.

Two doors, the same calls:

```bash
claude mcp add --transport http elo https://elopros.com/mcp/   # the `assets` tool
elo assets help                                                 # the launcher, https://elopros.com/download.html
```

## Use one in a project

1. **Find it.** `assets(q='low poly trees', engine='godot')`, or
   `elo assets search low poly trees --engine godot`. Each row has an id, a
   price, a kind, the engines and the licence.
2. **Read it.** `assets(what='get', id='…')`: the files, their sha256s, the
   licence, the credit line, and `buy` (free, open, or why not yet).
3. **Add it.** In the project folder, `elo assets add <id>`. It works out the
   engine from the project (`project.godot`, a bevy `Cargo.toml`, a Unity
   `Assets/`), puts the files in that engine's folder (`assets/elo/<id>/` for
   Godot and Bevy, `Assets/Elo/<id>/` for Unity), checks every sha256 and pins
   the asset in `elo-assets.lock`. Commit the lock.
   Without the launcher: `assets(what='install', id='…', engine='godot')`
   returns the files, the folder and the credit line; download each file and
   check its sha256 yourself. Bytes that differ are not what the author
   published: delete them.
4. **Credit it.** `elo assets credits` writes `elo-credits.md`. Ship it with
   the game.
5. **Keep it current.** `elo assets outdated`, then `elo assets update`.
   `elo assets sync` rebuilds the files from the lock on another machine.

A priced asset installs only for a wallet that holds its licence. Pass that
wallet (`--wallet 0x…`, or `wallet=` over MCP). If the read says buying is
not open yet, tell the person; never fetch a priced asset's files around it.

## Make one and list it

### Without GitHub: upload it

The quickest way, and the one to use unless the person wants a repo. You need
a wallet that can sign (EIP-191 `personal_sign`): the person's, or your own
if you have one. The wallet that signs is the author and the payee.

**With a shell, it is one command.** Put the files in a folder with a
`preview.png` (or `.jpg`) for the card, then:

```bash
elo assets upload ./wood-crate --about "A wooden crate, 400 triangles" \
  --license CC0-1.0 --tags crate,props
```

It writes `elo-asset.json` into the folder (every file's sha256 and size),
signs once with the launcher's wallet (`elo account new`, or a browser wallet
paired with `elo pair`), uploads only what the store does not already have,
and prints the page and the `elo assets add` line. To update, change the
files and run it again with `--version 1.1.0`: the manifest in the folder
keeps the rest. `elo assets unlist wood-crate` takes it off the shelf (kept;
the next upload brings it back). `--dry-run` checks without signing.

**Over MCP or plain HTTP**, step by step:

1. **Make the files**, as below, into a folder, plus a picture for the card
   (`preview.png` or `.jpg`, under 2 MB).
2. **Prepare.** Write the manifest without any `url` (we pin the files and
   write one): `{"elo_asset": 1, "id", "name", "about", "kind", "version",
   "license", "engines", "price_cents", "preview": "preview.png", "files":
   [{"name", "sha256", "size"}], "made_with": {…}}`. Then
   `assets(what='prepare', wallet='0x…', manifest={…})`. It checks every
   field, names what is wrong, and returns `sign_this` and `at`.
3. **Sign** `sign_this` exactly as returned, with the wallet. Ask the person
   to sign it if it is their wallet; never ask for their key.
4. **Upload.** `assets(what='upload', wallet, manifest, signature, at)` (the
   same manifest) returns `send`: one `put` url per file still needed, plus
   the preview. PUT each file's bytes there:
   `curl -T crate.glb https://elopros.com<put>`. Each is checked against its
   sha256 before it is pinned; the last one lists the asset.
   With no shell, `assets(what='send', upload=<token>, file=<name>,
   data_b64=<base64>)` sends one file of up to 2.5 MB through the tool.
5. **Update:** a new `version`, prepare, sign and upload again. Files the
   wallet already uploaded are not sent twice, and the picture is kept while
   its name stays the same.
6. **Unlist:** `assets(what='unlist', id, wallet)` returns the text to sign;
   call it again with `signature` and `at`.

A plugin or template cannot be uploaded: code lists only from GitHub, over an
open licence (below).

### From a GitHub release

Example: a person asks for "a low-poly wooden crate for my Godot game, and
put it on the store."

1. **Make the files.** With a Blender MCP server, model it and export to a
   folder: `crate.glb` (glTF binary works in Godot, Bevy, Unity, Unreal and
   three.js), and render a `preview.png` (16:9 reads best on the card). Keep
   file names plain: letters, digits, `.`, `_`, `+`, `-`.
2. **Make the repo and the release.** A public GitHub repo, say
   `you/wood-crate`, with a LICENSE file. Create release `v1.0.0` and upload
   every file in the folder to it (`gh release create v1.0.0 out/*`).
3. **Write the manifest.** From the repo root:
   ```bash
   elo assets init out --repo https://github.com/you/wood-crate --tag v1.0.0 \
     --kind model --license CC0-1.0 --engine godot --engine any \
     --about "A wooden crate, 400 triangles, one 512px texture"
   ```
   It writes `elo-asset.json` with every file's url, sha256 and size. Then
   edit it: add `"preview": "preview.png"` (and commit `preview.png` to the
   repo, not only the release), and say where a model was used:
   `"made_with": {"parts": ["art"], "note": "modelled by an agent in Blender"}`.
   Leave it out and the card says *not declared*; `{"parts": []}` means none.
   No launcher? Copy https://github.com/AnthonE/elo-pros-launcher/blob/main/sdk/repo/elo-asset.json
   and fill it in, with `sha256sum` for each file.
4. **Check it.** `elo assets check --repo https://github.com/you/wood-crate`,
   or `assets(what='check', repo='…', manifest={…})`. It names any field that
   is wrong and lists warnings. Nothing is listed by a check.
5. **Commit and publish.** Commit `elo-asset.json` to the default branch,
   then `elo assets publish --repo https://github.com/you/wood-crate` or
   `assets(what='publish', repo='…')`. The store reads the committed file;
   the request only says where to look.
6. **Update.** New release, new `version`, new files in the manifest, commit,
   publish again.

### Plugins and templates

A `plugin` or `template` runs code in the builder's project, so it lists only
over an open licence: the repo needs a LICENSE file (MIT or Apache-2.0 are
the usual). Its card and every install say it runs code. When you add one to
a person's project, tell them so.

### Price and payee

`price_cents` is 0 to 99. To be paid, add `"payee": "0x…"` — the person's
wallet, never one you hold for them unless they asked. Until a payee is named
the gamedev wallet holds what the asset earns. A sale pays the payee in the
buyer's own transaction. No refunds, and the licence is the buyer's to keep or
sell.

### What may be listed

What you made, or what your licence lets you sell, with `source` naming where
it came from. Re-listing someone else's free pack as your own breaks the
posted terms; re-listing it at 0¢ with the source named and the licence kept
does not. Nobody reviews an asset; the house takes one down only for the law
or the posted terms.

## Your shelf

`assets(what='shelf', owner='your-github-account')` (or a wallet, `0x…`)
lists every asset an author publishes: live, gone or taken down, with why, the warnings to fix and
the adds this week. The same list is on https://elopros.com/assets.html.
