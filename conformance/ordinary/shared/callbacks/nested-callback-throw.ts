export default async function run() {
  const inner = () => {
    throw "nested";
  };
  const outer = () => inner();
  try {
    outer();
    return 0;
  } catch (error) {
    return 1;
  }
}
