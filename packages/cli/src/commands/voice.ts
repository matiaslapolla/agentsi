import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { codeRoot, voiceDir } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";

/** Plugin hooks run from a cached copy of the plugin, so the state file tells them where the voice package is. */
export function setVoice(sessionId: string, on: boolean, dir = voiceDir(), code = codeRoot): void {
  const file = join(dir, `${sessionId}.json`);
  if (!on) return rmSync(file, { force: true });
  mkdirSync(dir, { recursive: true });
  writeFileSync(file, `${JSON.stringify({ code }, null, 2)}\n`);
}

export const voiceIsOn = (sessionId: string, dir = voiceDir()): boolean => existsSync(join(dir, `${sessionId}.json`));

const USAGE = "agentsi voice on|off|status";

export const voice: Command = {
  name: "voice",
  summary: "Turn voice mode on or off for the current Claude Code session",
  usage: USAGE,
  async run(argv) {
    const [action, ...extra] = argv;
    if (!action || extra.length || !["on", "off", "status"].includes(action)) throw new UsageError(USAGE);
    const sessionId = process.env.CLAUDE_CODE_SESSION_ID?.trim();
    if (!sessionId || !/^[\w-]+$/.test(sessionId)) {
      console.error("Run this command from inside a Claude Code session: CLAUDE_CODE_SESSION_ID is not set.");
      return 1;
    }
    if (action !== "status") setVoice(sessionId, action === "on");
    console.log(`Voice mode is ${voiceIsOn(sessionId) ? "on" : "off"} for this session.`);
    if (action === "on") console.log(`Transcripts: ${join(voiceDir(), "transcripts")}`);
  },
};
