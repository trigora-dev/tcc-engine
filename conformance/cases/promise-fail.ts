import { effect } from "@tcc-engine/primitives";
export default async function run() {
  try {
    const pair = await Promise.all([
      effect("bad", async () => 1),
      effect("sibling", async () => 2),
    ]);
    return pair;
  } catch (e) {
    return "caught";
  }
}
