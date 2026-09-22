function pad(value, fallback = value) {
  if (fallback === undefined) {
    return 0;
  }
  return fallback;
}

export default async function run(a, b = a) {
  return pad(b) + pad(undefined, 4);
}
