export function maybeCrash(hook: string, detail?: string): void {
  const want = process.env.TCC_CRASH_AT;
  if (!want) {
    return;
  }
  const n = bump(hook);
  if (want === hook || want === `${hook}:${n}` || (detail !== undefined && want === `${hook}:${detail}`)) {
    process.kill(process.pid, "SIGKILL");
  }
}

const counts = new Map<string, number>();

function bump(hook: string): number {
  const n = (counts.get(hook) ?? 0) + 1;
  counts.set(hook, n);
  return n;
}
