// Does this wallet own a copy? The check a game server runs at join.
//
//     node examples/owns.mjs gates 0xWALLET
//
// Kick on a definite false only: could_not_look is not "does not own".
import { Client } from "@elopros/sdk";

const [slug, wallet] = process.argv.slice(2);
const got = await new Client().owns(slug ?? "gates", wallet ?? "");
if (got.kind === "answer") console.log(got.value ? "owns a copy" : "no copy");
else if (got.kind === "could_not_look") console.log(`could not look (${got.error}) — let them play`);
else console.log(`refused: ${got.error}`);
