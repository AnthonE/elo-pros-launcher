/* The shared fixtures, sdk/testdata/. */
import { readFileSync } from "node:fs";

export const fx = (name) => JSON.parse(readFileSync(new URL(`../../testdata/${name}`, import.meta.url), "utf8"));
