export default async function run() {
  try {
    const f = () => {
      throw "boom";
    };
    f();
    return 0;
  } catch (error) {
    return 1;
  }
}
