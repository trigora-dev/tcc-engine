import { effect } from "@tcc-engine/primitives";
export default async function run() {
  const result = await effect("child_work", async () => 7);
  return result;
}
