import { sleep, waitForEvent } from "@tcc-engine/primitives";
export default async function run() {
  return await Promise.race([waitForEvent("ready"), sleep(60_000)]);
}
