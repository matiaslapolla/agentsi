export interface Command {
  name: string;
  summary: string;
  usage: string;
  /** Resolves to a process exit code (undefined means 0). */
  run(argv: string[]): Promise<number | void>;
}

export class UsageError extends Error {}
