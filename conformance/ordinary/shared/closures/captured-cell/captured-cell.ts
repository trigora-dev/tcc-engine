export default async function run() {
  let x = 1;
  const read = () => x;
  x = 2;
  return read();
}
