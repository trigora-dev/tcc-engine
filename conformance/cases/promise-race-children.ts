import { invoke } from "@tcc-engine/primitives";
export default async function run() {
  return await Promise.race([invoke("child"), invoke("child")]);
}
