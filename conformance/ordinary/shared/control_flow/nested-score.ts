function score(a, b, c) {
  return (a + b) * c - a / 2;
}

export default async function run() {
  return score(4, 2, 3);
}
