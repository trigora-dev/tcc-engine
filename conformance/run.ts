import path from "node:path";
import { pathToFileURL } from "node:url";

import type { HostConformanceDriver } from "./driver.ts";
import { runKit } from "./runner.ts";

async function loadDriver(): Promise<HostConformanceDriver> {
  const args = process.argv.slice(2);
  const index = args.indexOf("--driver");
  if (index < 0) {
    const { createNodeSqliteDriver } = await import("./drivers/node-sqlite.ts");
    return createNodeSqliteDriver();
  }
  const spec = args[index + 1];
  if (!spec) {
    throw new Error("--driver requires a module path");
  }
  const href = pathToFileURL(path.resolve(spec)).href;
  const mod = (await import(href)) as {
    createDriver?: () => HostConformanceDriver | Promise<HostConformanceDriver>;
    default?:
      | (() => HostConformanceDriver | Promise<HostConformanceDriver>)
      | HostConformanceDriver;
  };
  const factory = mod.createDriver ?? mod.default;
  if (!factory) {
    throw new Error(`${spec} must default-export createDriver()`);
  }
  return typeof factory === "function" ? await factory() : factory;
}

const driver = await loadDriver();
await runKit(driver);
console.log("host-conformance-v1 ok");
