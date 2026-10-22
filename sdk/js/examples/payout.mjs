// Write a closed wipe's payout file the way the meter reads it.
//
//     node examples/payout.mjs /tmp
//
// A game server writes this when a wipe closes and serves it at
// /payout/<wipe>.json beside status.json.
import { join } from "node:path";
import { payoutDoc, payoutFileName, writePayout } from "@elopros/sdk";

const doc = payoutDoc(3, [
  { address: "0x2222222222222222222222222222222222222222", amount: 120, ticker: "JUNK", for: ["EXTRACTED 120"] },
  { address: "0x3333333333333333333333333333333333333333", amount: 40, ticker: "JUNK", for: ["EXTRACTED 40"] },
]);
const path = join(process.argv[2] ?? ".", payoutFileName("island", doc.wipe));
await writePayout(path, doc);
console.log(`wrote ${path}`);
