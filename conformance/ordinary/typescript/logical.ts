export default async function run() {
  const user = { active: true, name: "ada" };
  const label = user.active && user.name;
  const fallback = user.missing || "guest";
  return { label, fallback };
}
