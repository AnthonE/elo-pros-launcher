// A wallet's friends and buddies, and who is playing what.
//
//     node examples/friends.mjs 0xYOURWALLET
import { Client, friendLabel } from "@elopros/sdk";

const wallet = process.argv[2];
if (!wallet) {
  console.error("usage: node examples/friends.mjs 0xWALLET");
  process.exit(2);
}
const got = await new Client().friends(wallet);
if (!got.ok) {
  console.error(`${got.kind}: ${got.error}`);
  process.exit(1);
}
const view = got.value;
if (!view.presence.reachable) console.log(`(presence could not be read: ${view.presence.why})`);
for (const f of [...view.friends, ...view.buddies]) {
  const tag = f.buddy ? " [buddy]" : "";
  console.log(f.playing
    ? `${friendLabel(f)}${tag} — playing ${f.playing.game} on ${f.playing.server_name ?? "?"}`
    : `${friendLabel(f)}${tag} — ${f.online ? "online" : "offline"}`);
}
console.log(`${view.count} friends, ${view.buddies.length} buddies, ${view.asks_in.length} asks waiting`);
