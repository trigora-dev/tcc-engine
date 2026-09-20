import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const kit = path.join(root, "hosts/node/src/kit.test.ts");
const result = spawnSync(
  process.execPath,
  ["--experimental-sqlite", "--experimental-strip-types", "--test", kit],
  { cwd: root, stdio: "inherit", env: process.env },
);
process.exit(result.status === null ? 1 : result.status);
