import { effect } from "@tcc-engine/primitives";
export default async function run() {
  let step = 1;
  let seen = 0;
  while (step) {
    seen = await Promise.race([effect("tick", async () => 7)]);
    if (step === 1) {
      step = 2;
    } else {
      step = 0;
    }
  }
  return seen;
}
