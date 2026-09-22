import { effect } from "@tcc-engine/primitives";
export default async function run() {
  try {
    return await Promise.race([
      effect("bad", async () => 1),
      effect("sibling", async () => 2),
    ]);
  } catch (e) {
    return "caught";
  }
}
