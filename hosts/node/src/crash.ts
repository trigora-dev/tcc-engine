export function maybeCrash(hook: string): void {
  if (process.env.TCC_CRASH_AT === hook) {
    process.kill(process.pid, "SIGKILL");
  }
}
