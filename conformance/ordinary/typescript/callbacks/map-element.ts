export default async function run() {
  const xs = [{ n: 1 }, { n: 2 }];
  xs.map((item) => {
    item.n += 1;
    return item.n;
  });
  return xs[0].n;
}
