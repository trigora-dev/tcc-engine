import { effect } from "@tcc-engine/primitives";

function nextAttempt(attempt) {
  return attempt + 1;
}

export default async function run() {
  let attempt = 0;
  let accepted = false;
  while (!accepted && attempt < 3) {
    attempt = nextAttempt(attempt);
    const status = await effect("try-send", async () => send());
    if (status === "ok") {
      accepted = true;
    }
  }
  return attempt;
}
