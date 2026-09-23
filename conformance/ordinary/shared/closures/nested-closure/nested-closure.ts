export default async function run() {
  const a = 2;
  const f = () => {
    const b = 3;
    const g = () => a + b;
    return g();
  };
  return f();
}
