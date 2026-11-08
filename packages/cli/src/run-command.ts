import { UsageError, type Command } from "./command.ts";

export interface CommandRun {
  status: number;
  stdout: string;
  stderr: string;
}

/**
 * Runs a command in this process, as `main` does: the console output is captured and an error becomes a status,
 * so a test can use the in-process test database that `getPool()` points at. A subprocess cannot reach it.
 */
export async function runCommand(command: Command, argv: string[]): Promise<CommandRun> {
  const out: string[] = [];
  const err: string[] = [];
  const { log, error } = console;
  console.log = (...a: unknown[]) => void out.push(a.join(" "));
  console.error = (...a: unknown[]) => void err.push(a.join(" "));
  let status = 0;
  try {
    status = (await command.run(argv)) ?? 0;
  } catch (e) {
    if (e instanceof UsageError) err.push(`${e.message}\nUsage: ${command.usage}`);
    else err.push(`agentsi ${command.name}: ${(e as Error).message}`);
    status = e instanceof UsageError && e.message.startsWith("No product") ? 2 : 1;
  } finally {
    console.log = log;
    console.error = error;
  }
  const text = (lines: string[]) => lines.map((l) => l + "\n").join("");
  return { status, stdout: text(out), stderr: text(err) };
}
