export default async function run() {
  const xs = [1, 2, 3];
  const sum = xs.reduce((acc, n) => acc + n, 0);
  const bare = xs.reduce((acc, n) => acc + n);
  const empty = [].reduce((acc, n) => acc + n, undefined);
  return sum + bare + (empty === undefined ? 10 : 0);
}
