import { waitForEvent } from "@tcc-engine/primitives";

export default async function run() {
  const items = [2, 5, 9];
  let total = 0;
  for (const n of items) {
    total += n * 2;
    if (total > 10) {
      const decision = await waitForEvent("review");
      if (decision === "stop") {
        return total;
      }
    }
  }
  return total;
}
