import { effect, waitForEvent } from "@tcc-engine/primitives";
export default async function run() {
  const flag = await effect("generate", async () => 1);
  if (flag) {
    const approval = await waitForEvent("approved");
    return approval;
  }
  return 0;
}
