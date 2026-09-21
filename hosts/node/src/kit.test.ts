import test from "node:test";

import { createNodeSqliteDriver } from "../../../conformance/drivers/node-sqlite.ts";
import { runKit } from "../../../conformance/runner.ts";

test("host conformance v1", async () => {
  await runKit(createNodeSqliteDriver());
});
