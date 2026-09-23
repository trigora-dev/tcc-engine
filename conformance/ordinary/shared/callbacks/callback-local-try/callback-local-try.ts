export default async function run() {
  const f = () => {
    try {
      throw "inner";
    } catch (error) {
      return 1;
    }
  };
  try {
    return f();
  } catch (error) {
    return 0;
  }
}
