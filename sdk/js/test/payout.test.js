import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { ActError, checkPayout, checkPayoutRow, payoutDoc, payoutFileName, writePayout } from "../index.js";
import { fx } from "./_testdata.js";

test("every good row is owed", () => {
  const good = fx("payout.json");
  assert.deepEqual(checkPayout(good), []);
  assert.deepEqual(payoutDoc(good.wipe, good.payouts).payouts[0].for, ["EXTRACTED 120"]);
});

test("every bad row and file is refused", () => {
  const bad = fx("payout_bad.json");
  for (const b of bad.payouts) {
    const why = checkPayoutRow(b.row);
    assert.ok(why && why.startsWith(b.why), `${JSON.stringify(b.row)}: ${why}`);
  }
  for (const b of bad.files) assert.ok(checkPayout(b.doc).length > 0, b.why);
  assert.throws(() => payoutDoc(0, []), ActError);
});

test("a payout file is written whole, or not at all", async () => {
  const dir = await mkdtemp(join(tmpdir(), "elo-sdk-payout-"));
  try {
    const doc = payoutDoc(3, [{ address: "0x2222222222222222222222222222222222222222", amount: 120,
      ticker: "JUNK", for: ["EXTRACTED 120"] }]);
    const path = join(dir, payoutFileName("island", 3));
    await writePayout(path, doc);
    assert.deepEqual(JSON.parse(await readFile(path, "utf8")), doc);
    await assert.rejects(writePayout(join(dir, "bad.json"), { wipe: 0, payouts: [] }), ActError);
    assert.deepEqual(await readdir(dir), ["island.payout-3.json"]);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
