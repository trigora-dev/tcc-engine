import { effect } from "@tcc-engine/primitives";
export default async function run() {
  let step = 1;
  let seen = 0;
  while (step) {
    const [value] = await Promise.all([effect("tick", async () => 7)]);
    seen = value;
    if (step === 1) {
      step = 2;
    } else {
      step = 0;
    }
  }
  return seen;
}
