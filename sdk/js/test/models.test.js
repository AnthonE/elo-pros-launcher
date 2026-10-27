import test from "node:test";
import assert from "node:assert/strict";

import { classify, friendLabel } from "../index.js";
import { fx } from "./_testdata.js";

const answer = (body) => {
  const got = classify(200, JSON.stringify(body));
  assert.equal(got.kind, "answer");
  return got.value;
};

test("the friends view, with buddies", () => {
  const v = answer(fx("friends_view.json"));
  assert.equal(v.count, 3);
  assert.equal(v.friends.length, 3);
  assert.equal(v.buddies.length, 1);
  assert.deepEqual(v.buddies[0].buddy, { id: "pip", kind: "buddy" });
  assert.ok(v.friends.every((f) => !f.buddy));
  assert.equal(friendLabel(v.friends[0]), "ash");
  assert.equal(friendLabel(v.friends[2]), "0x4444…4444");
  assert.equal(v.friends[0].playing.join, "elo://join/gates/game.elopros.com:61234");
});

test("ticket ownership: a failed chain read is could-not-look", () => {
  const o = fx("ticket_of.json");
  assert.equal(answer(o.owned).entitled, true);
  const down = classify(o.could_not_look.status, JSON.stringify(o.could_not_look.body));
  assert.equal(down.kind, "could_not_look");
  assert.match(down.error, /could not look/);
  const card = answer(fx("ticket.json").ticketed);
  assert.equal(card.price_usd, 9.99);
  assert.equal(card.all_rails[0].buyable, true);
});

test("items: an index that timed out is not an empty wallet", () => {
  const i = fx("items_of.json");
  const held = answer(i.held);
  assert.equal(held.count, 3);
  assert.equal(held.items[2].via, "vault");
  assert.equal(classify(200, JSON.stringify(i.could_not_look)).kind, "could_not_look");
  assert.equal(answer(i.not_configured).configured, false);
});

test("leaderboards", () => {
  const b = fx("leaderboard.json");
  const island = answer(b.published).boards[0];
  assert.equal(island.entries[0][island.sort_key], 1200);
  assert.equal(classify(200, JSON.stringify(b.unpublished)).kind, "could_not_look");
});

test("who and face parse", () => {
  assert.equal(answer(fx("who_fixture.json").who).found.agent, "Moss");
  assert.equal(answer(fx("face.json").set).name, "ash");
});

test("guild tags, a wallet's guilds, and the face's guild", () => {
  const g = fx("guilds.json");
  const tags = answer(g.tags).tags;
  assert.equal(tags["0x" + "b2".repeat(20)].tag, "OWL");
  assert.equal(tags["0x" + "c3".repeat(20)], null);
  const of = answer(g.of);
  assert.equal(of.worn, "OWL");
  assert.equal(of.guilds[0].emblem.shape, "hex");
  assert.equal(answer(fx("face.json").set).guild.tag, "OWL");
  assert.equal(answer(fx("face.json").unset).guild, null);
});

test("the three outcomes stay apart", () => {
  assert.deepEqual(classify(404, '{"error":"no ask from that wallet"}'),
    { kind: "refused", ok: false, status: 404, error: "no ask from that wallet", body: { error: "no ask from that wallet" } });
  assert.equal(classify(404, '{"detail":"Not Found"}').error, "Not Found");
  assert.equal(classify(502, "<html>bad gateway</html>").kind, "could_not_look");
  assert.equal(classify(200, '{"reachable":false,"error":"timeout"}').error, "timeout");
});
