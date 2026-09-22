export default async function run() {
  const point = { x: 3, y: 4 };
  const { x, y } = point;
  return x * x + y * y;
}
