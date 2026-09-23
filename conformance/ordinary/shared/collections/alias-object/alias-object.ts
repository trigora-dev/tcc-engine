export default async function run() {
  const a = { x: 1 };
  const b = a;
  b.x = 2;
  return a.x;
}
