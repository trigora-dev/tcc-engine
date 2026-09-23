function classify(n) {
  if (n < 0) {
    return "neg";
  }
  if (n === 0) {
    return "zero";
  }
  return "pos";
}

export default async function run() {
  return classify(-3);
}
