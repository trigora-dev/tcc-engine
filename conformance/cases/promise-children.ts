import { invoke } from "@tcc-engine/primitives";
export default async function run() {
  const pair = await Promise.all([invoke("child"), invoke("child")]);
  return pair;
}
