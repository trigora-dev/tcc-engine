import { waitForEvent } from "@tcc-engine/primitives";

function calculate(a, b) {
  return a * b;
}

export default async function run() {
  const total = calculate(6, 7);
  await waitForEvent("continue");
  return total;
}
