import { waitForEvent } from "@tcc-engine/primitives";

export default async function run() {
  const multiplier = 3;
  const transform = (value) => value * multiplier;
  const value = await waitForEvent("go");
  return transform(value);
}
