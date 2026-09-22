import { waitForEvent } from "@tcc-engine/primitives";
export default async function run() {
  const pair = await Promise.all([
    waitForEvent("ready"),
    waitForEvent("ready"),
  ]);
  return pair;
}
