import { createNodeSqliteDriver } from "./drivers/node-sqlite.ts";
import { runKit } from "./runner.ts";

await runKit(createNodeSqliteDriver());
console.log("host-conformance-v1 ok");
