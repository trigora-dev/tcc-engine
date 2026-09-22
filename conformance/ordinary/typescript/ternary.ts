export default async function run() {
  const n = -2;
  const label = n > 0 ? "pos" : n < 0 ? "neg" : "zero";
  return label;
}
