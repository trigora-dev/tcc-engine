export default async function run() {
  let seen = 0;
  const f = () => {
    try {
      throw "x";
    } finally {
      seen = 1;
    }
  };
  try {
    f();
  } catch (error) {
    return seen;
  }
  return 0;
}
