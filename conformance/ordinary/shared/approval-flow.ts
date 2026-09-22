import { waitForEvent } from "@tcc-engine/primitives";

function withinBudget(amount, limit) {
  return amount <= limit;
}

export default async function run() {
  const amount = 40;
  const limit = 50;
  if (!withinBudget(amount, limit)) {
    return "rejected";
  }
  const decision = await waitForEvent("approval");
  if (decision === "yes") {
    return "approved";
  }
  return "rejected";
}
