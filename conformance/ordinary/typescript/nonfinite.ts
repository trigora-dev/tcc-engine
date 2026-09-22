export default async function run() {
  const inf = 1 / 0;
  const nan = 0 / 0;
  const sum = inf + 1;
  const neg = -inf;
  return { inf, nan, sum, neg };
}
