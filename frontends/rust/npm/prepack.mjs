import { chmodSync, copyFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const binary = path.resolve(here, "../../../target/release/tcc-rust-compile");
const vendor = path.join(here, "vendor");
mkdirSync(vendor, { recursive: true });
const dest = path.join(vendor, "tcc-rust-compile");
try {
  copyFileSync(binary, dest);
} catch (error) {
  throw new Error(
    `missing ${binary}. Build it with: cargo build -p tcc-rust-frontend --release`,
    { cause: error },
  );
}
chmodSync(dest, 0o755);
