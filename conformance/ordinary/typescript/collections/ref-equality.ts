export default async function run() {
  const a = { x: 1 };
  const b = a;
  const c = { x: 1 };
  const nan = 0 / 0;
  return {
    same: a === b,
    other: a === c,
    negZero: 0 === -0,
    nan: nan === nan,
  };
}
