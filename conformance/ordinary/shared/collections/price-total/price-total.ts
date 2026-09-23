export default async function run() {
  const price = 10;
  const quantity = 3;
  const tax = 2;
  const limit = 20;
  const user = { active: true };
  const subtotal = price * quantity;
  const total = subtotal + tax;
  if (total > limit && user.active) {
    return total;
  }
  return 0;
}
