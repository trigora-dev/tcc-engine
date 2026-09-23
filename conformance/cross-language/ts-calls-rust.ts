import { invoke } from "@tcc-engine/primitives";

export default async function run() {
  const value = await invoke("child", 2);
  return value + 1;
}
