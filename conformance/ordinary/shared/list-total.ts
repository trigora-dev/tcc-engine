export default async function run() {
  const items = [
    { price: 10, quantity: 2 },
    { price: 4, quantity: 3 },
  ];
  const limit = 20;
  const user = { active: true };
  let total = 0;
  for (const item of items) {
    total += item.price * item.quantity;
  }
  if (total > limit && user.active) {
    return total;
  }
  return 0;
}
