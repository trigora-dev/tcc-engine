export default async function run() {
  const xs = [10, 20, 30];
  xs.push(40);
  xs[1] = xs[0] + 5;
  return {
    len: xs.length,
    second: xs[1],
    missing: xs[9],
    last: xs[xs.length - 1],
  };
}
