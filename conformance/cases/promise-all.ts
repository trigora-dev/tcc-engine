import { effect } from "@tcc-engine/primitives";
export default async function run() {
  const pair = await Promise.all([
    effect("a", async () => 1),
    effect("b", async () => 2),
  ]);
  return pair;
}
