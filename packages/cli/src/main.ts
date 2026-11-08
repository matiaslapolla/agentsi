import { closePool } from "@agentsi/core";
import { UsageError, type Command } from "./command.ts";
import { archive, restore } from "./commands/archive.ts";
import { ask } from "./commands/ask.ts";
import { audit } from "./commands/audit.ts";
import { backup } from "./commands/backup.ts";
import { check } from "./commands/check.ts";
import { data } from "./commands/data.ts";
import { decide } from "./commands/decide.ts";
import { decision } from "./commands/decision.ts";
import { desk } from "./commands/desk.ts";
import { doctorCommand } from "./commands/doctor.ts";
import { indexCommand } from "./commands/index.ts";
import { mark } from "./commands/mark.ts";
import { mcp } from "./commands/mcp.ts";
import { meet } from "./commands/meet.ts";
import { migrate } from "./commands/migrate.ts";
import { reflect } from "./commands/reflect.ts";
import { render } from "./commands/render.ts";
import { reset } from "./commands/reset.ts";
import { roles } from "./commands/roles.ts";
import { snapshot } from "./commands/snapshot.ts";
import { status } from "./commands/status.ts";
import { story } from "./commands/story.ts";
import { up } from "./commands/up.ts";
import { voice } from "./commands/voice.ts";
import { web } from "./commands/web.ts";
import { whoami } from "./commands/whoami.ts";

const commands: Command[] = [up, doctorCommand, migrate, data, backup, whoami, desk, indexCommand, status, snapshot, check, audit, meet, ask, decide, decision, story, mark, archive, restore, roles, reset, reflect, render, voice, web, mcp];

function help(): string {
  const width = Math.max(...commands.map((c) => c.name.length));
  return ["Usage: agentsi <command> [options]", "", ...commands.map((c) => `  ${c.name.padEnd(width)}  ${c.summary}`)].join("\n");
}

async function main(): Promise<number> {
  const [name, ...rest] = process.argv.slice(2);
  if (!name || name === "help" || name === "--help" || name === "-h") {
    console.log(help());
    return name ? 0 : 1;
  }
  const command = commands.find((c) => c.name === name);
  if (!command) {
    console.error(`Unknown command "${name}"\n\n${help()}`);
    return 1;
  }
  try {
    return (await command.run(rest)) ?? 0;
  } catch (err) {
    if (err instanceof UsageError) console.error(`${err.message}\nUsage: ${command.usage}`);
    else console.error(`agentsi ${name}: ${(err as Error).message}`);
    return err instanceof UsageError && err.message.startsWith("No product") ? 2 : 1;
  }
}

const code = await main();
await closePool();
process.exit(code);
