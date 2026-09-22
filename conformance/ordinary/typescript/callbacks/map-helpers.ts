function double(n) {
  return n * 2;
}

export default async function run() {
  const xs = [1, 2, 3];
  const fromHelper = xs.map(double);
  const fromArrow = xs.map((n) => n * 2);
  const same = double === double ? 1 : 0;
  return fromHelper[0] + fromHelper[2] + fromArrow[1] + same;
}
