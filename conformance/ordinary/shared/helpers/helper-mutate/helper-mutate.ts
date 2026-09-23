function update(x) {
  x.count += 1;
}

export default async function run() {
  const state = { count: 0 };
  update(state);
  return state.count;
}
