import { readFileSync } from "node:fs";

import { canonicalStringify } from "../frontends/typescript/src/canonical.ts";
import { compile } from "../frontends/typescript/src/compile.ts";

const filename = process.argv[2];
if (!filename) {
  throw new Error("usage: compile-ts.ts <file>");
}
const source = readFileSync(filename, "utf8");
process.stdout.write(canonicalStringify(compile(source, { filename })));
