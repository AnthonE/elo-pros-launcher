---
name: elo-look
description: Use when an AI agent (Claude Code, Codex, Cursor, Hermes, OpenClaw, any harness with a shell) should make a game look right — set up a reference board for the game (named views, each shot with no GPU, beside reference images and the operator's verdicts), run the look loop (change one thing, shoot, read the side-by-side sheet, repeat) and stop when the operator marks a view right. Works on any game whose frame a command can photograph headless; ships board.mjs, no dependencies but Node and Playwright.
license: MIT
metadata:
  version: "1.0.0"
  hermes:
    category: development
    tags: [games, art, gamedev, screenshots, references, blender]
  openclaw:
    category: development
    tags: [games, art, gamedev, screenshots, references, blender]
---

# elo-look — make a game look right, from references

How a look got made on Gates and Fireplace, as a loop any harness can run:

1. **References first.** Pictures of what it should look like, each with a
   note: what to take from it and what to ignore.
2. **The game photographs itself, with no GPU.** A headless browser on
   SwiftShader, or a native client on lavapipe under Xvfb, draws the real
   frame on a box with no graphics card. Slow (a second or more a frame) and
   right. Frame rates mean nothing; pictures mean everything.
3. **One sheet, both sides.** Each view's references, the frame now and the
   frame from the last pass in one row, with the same measurements under
   every picture.
4. **The operator's word is the bar.** A view is done when the person whose
   game it is marks it `right`. Numbers keep a floor; they never overrule a
   mark, and a judge that only reads numbers will chase them while the
   picture gets worse.

`scripts/board.mjs` runs all of it. Copy it into the game as
`tools/board.mjs`.

## Set up a board

`board/board.json`, beside the game:

```json
{
  "game": "My Game",
  "refs": "refs",
  "private_refs_env": "ELO_REF_DIR",
  "views": [
    {
      "id": "spawn-day",
      "what": "where a new player stands, at noon",
      "take": "the first thing a player sees should say what the game is",
      "shoot": ["node", "tools/shot.mjs", "--view", "spawn", "--hour", "12"],
      "png": "traces/spawn.png",
      "refs": []
    }
  ]
}
```

- **A view** is one fixed picture: the same seed, camera, hour and weather on
  every pass, so two passes compare. Pick the frames that matter: a new
  player's first look, a fight, a menu, a crowd. Put someone in the frame
  (bots, a demo scene): an empty world photographs as terrain.
- **`shoot`** is the game's own command that writes `png`, as an array (no
  shell). Views that share a command are shot by one run of it. If the game
  has no headless shooter yet, write one first: open the page in Playwright's
  Chromium with SwiftShader (`--use-angle=swiftshader`, or for WebGPU
  `--enable-unsafe-webgpu --use-webgpu-adapter=swiftshader`), wait for a
  ready flag the page sets, and `page.screenshot`. For a native client:
  Xvfb, lavapipe (`mesa-vulkan-drivers`) and the game's own screenshot key.
- **Cheap pictures before expensive ones.** If a full shot takes minutes,
  write a tool that answers a taste question in seconds (every candidate pose
  or colour on one sheet) and shoot the full view to confirm.

Existing boards to copy from: Fireplace's `board/board.json` (demo scenes
from `tools/party-shot.mjs`, the couch from `tools/theme-shots.mjs`).

## References

```bash
node tools/board.mjs ref spawn-day ~/refs/meadow.jpg --take "soft noon light, long grass to the horizon" --ignore "the UI" --licence CC0 --source https://…
node tools/board.mjs ref spawn-day ~/refs/other-game.png --take "how far you can see" --private
```

- A reference is a **target for the look, never an asset**. Nothing is traced,
  copied or sampled from one.
- One that is ours, CC0 or CC-BY goes in `board/refs/` with its licence.
  **Anything else is `--private`**: the file stays in the directory
  `$ELO_REF_DIR` names, outside the repo, and only its name and what to take
  from it are written down. A public repo that carries another studio's
  screenshots is redistributing them.
- Ask the operator for references before the first pass. Three good ones
  per view beat ten.

## The loop

```bash
node tools/board.mjs look                 # shoot every view, then the sheet
node tools/board.mjs look --views spawn-day
node tools/board.mjs status               # each view's last mark, and how many passes since
```

`look` prints the sheet's path (`board/passes/<stamp>/sheet.png`) and, per
view, Rec.601 luma over the frame, the sky band (top 25% of rows), the near
band (bottom 35%) and chroma (max minus min channel), each 0–255, with now's
distance from the references' median. `stats.json` beside it has the same
numbers. Then:

1. **Read the sheet**, the picture itself, not only the numbers. Say what you
   see in the frame against the references and the view's `take`.
2. **Change one thing** toward the references (a light, a palette, a model, a
   camera), in the game's code or assets.
3. **`look` again** and compare now against the last pass on the same row.
   If it got worse, undo it.
4. **Show the operator the sheet** when a view looks closer, and ask.

## Marks: the operator's word

```bash
node tools/board.mjs mark spawn-day wrong "the grass is too yellow"
node tools/board.mjs mark spawn-day right "keep this"
node tools/board.mjs mark spawn-day note "try evening too"
```

- `board/marks.jsonl` is append-only. **Record a mark only when the operator
  said it**, in their words. Never mark your own work `right`, never edit or
  delete a mark, and never change a view's `take` or a reference's note to
  make a frame pass.
- A view whose latest mark is `right` is done: leave it unless the operator
  asks again. Its frame is now worth keeping as a reference (`--licence ours`).
- A `wrong` mark's words are the next pass's goal.

## What to commit

`board/board.json`, `board/refs/` (ours and CC0 only) and
`board/marks.jsonl`. Not `board/passes/` (add it to `.gitignore`).

## Making the assets the board asks for

When the fix is a new model, texture or sound rather than code:
`elo-assets` finds one at 99¢ or less with its licence and credit written, or
makes one (a model in Blender through a Blender MCP server). Record the tool
and the prompt beside a generated asset so its licence is a fact anyone can
read.
