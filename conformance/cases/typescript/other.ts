import { effect, waitForEvent } from "@tcc-engine/primitives";
export default async function run() {
  const result = await effect("generate", async () => 99);
  const approval = await waitForEvent("approved");
  return { result, approval };
}
