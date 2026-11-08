// S024: voice mode hooks. The hook script is plugin/hooks/voice-mode.mjs.
// Contract used by these tests:
//   node voice-mode.mjs prompt   (UserPromptSubmit, payload on stdin)
//   node voice-mode.mjs stop     (Stop, payload on stdin)
//   AGENTSI_HOME is the AgentSI data folder; `agentsi voice on` writes <data>/voice/<session id>.json for one session.
//   The engine is ONE `agentsi-voice say pm <dir>` process per reply (S025): one sentence per stdin line, it writes
//   <dir>/<n>.wav and prints `WAV <path>` for each. The player is `pw-play`. Both are found on PATH.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";

const HOOK = new URL("./voice-mode.mjs", import.meta.url).pathname;
const REPO = resolve(new URL("../..", import.meta.url).pathname);
const SYSTEM_PATH = "/usr/bin:/bin";

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

async function until(fn, ms = 5000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (fn()) return true;
    await sleep(50);
  }
  return fn();
}

function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function files(dir) {
  const out = [];
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...files(p));
    else out.push(p);
  }
  return out;
}

function script(path, body, node = false) {
  writeFileSync(path, node ? `#!${process.execPath}\n${body}\n` : `#!/bin/sh\n${body}\n`);
  chmodSync(path, 0o755);
}

// A sandbox with a data folder, a fake engine and a fake player on PATH.
function setup({ engineFails = false, playerFails = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), "voice-mode-"));
  const data = join(root, "data");
  const bin = join(root, "bin");
  const calls = join(root, "engine-calls.log");
  const plays = join(root, "player-plays.log");
  mkdirSync(data);
  mkdirSync(bin);
  // Every sentence the engine gets is one line of the calls log: its pid, its argv and the sentence.
  if (engineFails) script(join(bin, "agentsi-voice"), `echo "engine boom: model not found" >&2\nexit 3`);
  else
    script(
      join(bin, "agentsi-voice"),
      `const fs = require("fs"), path = require("path");
const argv = process.argv.slice(2), dir = argv[argv.length - 1];
const wav = Buffer.alloc(44 + 3200);
wav.write("RIFF", 0); wav.writeUInt32LE(36 + 3200, 4); wav.write("WAVEfmt ", 8); wav.writeUInt32LE(16, 16);
wav.writeUInt16LE(1, 20); wav.writeUInt16LE(1, 22); wav.writeUInt32LE(16000, 24); wav.writeUInt32LE(32000, 28);
wav.writeUInt16LE(2, 32); wav.writeUInt16LE(16, 34); wav.write("data", 36); wav.writeUInt32LE(3200, 40);
let buf = "", n = 0;
process.stdin.on("data", (d) => (buf += d));
process.stdin.on("end", () => {
  fs.mkdirSync(dir, { recursive: true });
  for (const line of buf.split("\\n").map((l) => l.trim()).filter(Boolean)) {
    fs.appendFileSync(${JSON.stringify(calls)}, "PID[" + process.pid + "] ARGV[" + argv.join(" ") + "] LINE[" + line + "]\\n");
    const p = path.join(dir, n++ + ".wav");
    fs.writeFileSync(p, wav);
    process.stdout.write("WAV " + p + "\\n");
  }
});`,
      true,
    );
  script(
    join(bin, "pw-play"),
    playerFails ? `echo "player boom: no sound card" >&2\nexit 4` : `echo "PLAY $*" >> "${plays}"\nexit 0`,
  );
  const env = { PATH: `${bin}:${SYSTEM_PATH}`, AGENTSI_HOME: data, HOME: root, NO_COLOR: "1" };
  return {
    root,
    data,
    bin,
    calls,
    plays,
    env,
    state(enabled, sessionId) {
      const file = join(data, "voice", `${sessionId}.json`);
      if (!enabled) return rmSync(file, { force: true });
      mkdirSync(join(data, "voice"), { recursive: true });
      writeFileSync(file, JSON.stringify({ code: REPO }));
    },
    run(event, payload, extraEnv = {}) {
      return spawnSync(process.execPath, [HOOK, event], {
        input: JSON.stringify(payload),
        encoding: "utf8",
        env: { ...env, ...extraEnv },
        timeout: 15000,
      });
    },
    engineCalls() {
      return existsSync(calls) ? readFileSync(calls, "utf8").split("\n").filter(Boolean) : [];
    },
    cleanup() {
      rmSync(root, { recursive: true, force: true });
    },
  };
}

function transcript(root, turns, name = "session.jsonl") {
  const path = join(root, name);
  const lines = [];
  for (const [prompt, reply] of turns) {
    lines.push(JSON.stringify({ type: "user", message: { role: "user", content: prompt } }));
    lines.push(JSON.stringify({ type: "assistant", message: { role: "assistant", content: [{ type: "text", text: reply }] } }));
  }
  writeFileSync(path, lines.join("\n") + "\n");
  return path;
}

const REPLY = [
  "The build passes now. I fixed two tests.",
  "",
  "## Heading-words",
  "",
  "I changed packages/core/src/replay.ts to do it, with **bold** care.",
  "",
  "```ts",
  "const secretCode = 42;",
  "```",
  "",
  "On screen only",
  "",
  "Tail-details: a long table that must stay on screen.",
].join("\n");

test("voice mode off: both hooks add no rules, speak nothing, write no transcript, exit 0", () => {
  for (const mode of ["missing", "off"]) {
    const s = setup();
    try {
      if (mode === "off") s.state(false, "off-1");
      const t = transcript(s.root, [["hello there", REPLY]]);
      const p = s.run("prompt", { session_id: "off-1", prompt: "hello there", hook_event_name: "UserPromptSubmit" });
      assert.equal(p.status, 0, `${mode}: ${p.stderr}`);
      assert.equal(p.stdout.trim(), "", `${mode}: prompt hook printed output`);
      const st = s.run("stop", { session_id: "off-1", transcript_path: t, hook_event_name: "Stop" });
      assert.equal(st.status, 0, `${mode}: ${st.stderr}`);
      assert.equal(st.stdout.trim(), "", `${mode}: stop hook printed output`);
      assert.deepEqual(s.engineCalls(), [], `${mode}: engine was called`);
      assert.equal(existsSync(s.plays), false, `${mode}: player was started`);
      assert.deepEqual(
        files(s.data).filter((f) => f.endsWith(".md")),
        [],
        `${mode}: a transcript was written`,
      );
    } finally {
      s.cleanup();
    }
  }
});

test("voice mode on: the prompt hook adds the style rules and stops the player still running", async () => {
  const s = setup();
  let pid = 0;
  let other = null;
  try {
    s.state(true, "on-1");
    // The previous reply of this session is still playing: a long-running fake pw-play started by the stop hook.
    const pidFile = join(s.root, "player.pid");
    script(join(s.bin, "pw-play"), `echo $$ > "${pidFile}"\nwhile :; do sleep 1; done`);
    // A player of another app must survive.
    other = spawn("sleep", ["30"], { detached: true, stdio: "ignore" });
    other.unref();
    const t = transcript(s.root, [["first question", REPLY]]);
    assert.equal(s.run("stop", { session_id: "on-1", transcript_path: t, hook_event_name: "Stop" }).status, 0);
    assert.ok(await until(() => existsSync(pidFile)), "fake player did not start");
    pid = Number(readFileSync(pidFile, "utf8").trim());
    assert.ok(alive(pid), "fake player should be running before the hook");

    const r = s.run("prompt", { session_id: "on-1", prompt: "next question", hook_event_name: "UserPromptSubmit" });
    assert.equal(r.status, 0, r.stderr);
    const out = JSON.parse(r.stdout);
    const ctx = out.hookSpecificOutput?.additionalContext ?? "";
    assert.equal(out.hookSpecificOutput?.hookEventName, "UserPromptSubmit");
    assert.match(ctx, /On screen only/);
    assert.ok(ctx.length > "On screen only".length + 40, "the rules should say more than the fixed line");
    assert.ok(await until(() => !alive(pid)), "the fake player was not stopped");
    assert.ok(alive(other.pid), "the hook stopped a process it did not start");
  } finally {
    if (pid && alive(pid)) process.kill(pid, "SIGKILL");
    if (other && alive(other.pid)) process.kill(other.pid, "SIGKILL");
    s.cleanup();
  }
});

test("voice mode on: the stop hook speaks only the clean prose above the line, sentence by sentence, and returns at once", async () => {
  const s = setup();
  try {
    s.state(true, "on-2");
    const t = transcript(s.root, [["fix the build", REPLY]]);
    const started = Date.now();
    const r = s.run("stop", { session_id: "on-2", transcript_path: t, hook_event_name: "Stop" });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(Date.now() - started < 10000, "the hook should return without waiting for audio");
    assert.ok(await until(() => s.engineCalls().length >= 3), `engine calls: ${JSON.stringify(s.engineCalls())}`);
    const calls = s.engineCalls();
    for (const c of calls) {
      assert.match(c, /ARGV\[[^\]]*\bsay\b/, `not a say call: ${c}`);
      assert.match(c, /\bpm\b/, `pm voice missing: ${c}`);
    }
    const first = calls.filter((c) => c.includes("The build passes now"));
    const second = calls.filter((c) => c.includes("I fixed two tests"));
    const third = calls.filter((c) => c.includes("I changed"));
    assert.equal(first.length, 1, "sentence 1");
    assert.equal(second.length, 1, "sentence 2");
    assert.equal(third.length, 1, "sentence 3");
    assert.notEqual(first[0], second[0], "sentences must be separate lines");
    assert.equal(new Set(calls.map((c) => c.match(/PID\[(\d+)\]/)[1])).size, 1, "one engine process speaks the whole reply");
    const spoken = calls.join("\n");
    for (const banned of ["secretCode", "const ", "On screen only", "Tail-details", "packages/core", "replay.ts", "**", "```", "##"]) {
      assert.equal(spoken.includes(banned), false, `spoken text contains ${JSON.stringify(banned)}`);
    }
    assert.ok(await until(() => existsSync(s.plays)), "the player never played");
    assert.ok(await until(() => !files(s.data).some((f) => f.endsWith(".pid"))), "the speaker left its pid file behind");
  } finally {
    s.cleanup();
  }
});

test("voice mode on: a failing engine, player or transcript file never blocks and is logged", async () => {
  const cases = [
    { name: "engine", opts: { engineFails: true }, needle: /engine boom/ },
    { name: "player", opts: { playerFails: true }, needle: /player boom/ },
    { name: "transcript", opts: {}, needle: null },
  ];
  for (const c of cases) {
    const s = setup(c.opts);
    try {
      s.state(true, `fail-${c.name}`);
      const t = c.name === "transcript" ? join(s.root, "does-not-exist.jsonl") : transcript(s.root, [["go", REPLY]]);
      const r = s.run("stop", { session_id: `fail-${c.name}`, transcript_path: t, hook_event_name: "Stop" });
      assert.equal(r.status, 0, `${c.name}: ${r.stderr}`);
      const out = r.stdout.trim();
      if (out) assert.notEqual(JSON.parse(out).decision, "block", `${c.name}: the hook blocked`);
      const logged = await until(() => files(s.data).some((f) => f.endsWith(".log") && readFileSync(f, "utf8").trim() !== ""));
      assert.ok(logged, `${c.name}: no log file with the error in the data folder`);
      const log = files(s.data)
        .filter((f) => f.endsWith(".log"))
        .map((f) => readFileSync(f, "utf8"))
        .join("\n");
      if (c.needle) assert.match(log, c.needle, `${c.name}: error text missing from the log`);
      else assert.match(log, /does-not-exist|ENOENT/, "transcript error missing from the log");
    } finally {
      s.cleanup();
    }
  }
});

test("voice mode on: two turns give one Markdown transcript with both prompts, full replies and the spoken part marked", async () => {
  const s = setup();
  try {
    s.state(true, "two-turns");
    const reply2 = "Second answer is short. It has one more line.\n\nOn screen only\n\nSecond-tail-details stay written.";
    const sid = "two-turns";
    const t1 = transcript(s.root, [["first prompt from owner", REPLY]], "t1.jsonl");
    s.run("prompt", { session_id: sid, prompt: "first prompt from owner", hook_event_name: "UserPromptSubmit" });
    assert.equal(s.run("stop", { session_id: sid, transcript_path: t1, hook_event_name: "Stop" }).status, 0);
    const t2 = transcript(s.root, [["first prompt from owner", REPLY], ["second prompt from owner", reply2]], "t2.jsonl");
    s.run("prompt", { session_id: sid, prompt: "second prompt from owner", hook_event_name: "UserPromptSubmit" });
    assert.equal(s.run("stop", { session_id: sid, transcript_path: t2, hook_event_name: "Stop" }).status, 0);

    assert.ok(await until(() => files(s.data).some((f) => f.endsWith(".md"))), "no transcript written");
    const mds = files(s.data).filter((f) => f.endsWith(".md"));
    assert.equal(mds.length, 1, `expected one transcript, got ${mds.join(", ")}`);
    const md = readFileSync(mds[0], "utf8");
    for (const part of [
      "first prompt from owner",
      "second prompt from owner",
      "The build passes now",
      "secretCode", // the full reply keeps the code block
      "Tail-details",
      "Second answer is short",
      "Second-tail-details",
    ]) assert.ok(md.includes(part), `transcript lacks ${JSON.stringify(part)}`);
    assert.equal(md.split("first prompt from owner").length - 1, 1, "first prompt must not be duplicated");
    // The spoken part is marked: a blockquote or a labelled marker around the spoken text.
    assert.match(md, /^(>|\*\*Spoken|Spoken|<!-- ?spoken)/im, "no marker for the spoken part");
  } finally {
    s.cleanup();
  }
});

test("agentsi-voice doctor reports whether an audio player is found on PATH", () => {
  const uv = spawnSync("sh", ["-c", "command -v uv"], { encoding: "utf8" }).stdout.trim();
  assert.ok(uv, "uv is needed to run agentsi-voice");
  const voiceDir = join(REPO, "packages/voice");
  const run = (path) =>
    spawnSync(uv, ["run", "--offline", "--project", voiceDir, "agentsi-voice", "doctor"], {
      encoding: "utf8",
      env: { PATH: path, HOME: process.env.HOME, NO_COLOR: "1" },
      timeout: 60000,
    });
  const root = mkdtempSync(join(tmpdir(), "voice-doctor-"));
  try {
    const withPlayer = join(root, "with");
    const without = join(root, "without");
    mkdirSync(withPlayer);
    mkdirSync(without);
    script(join(withPlayer, "pw-play"), "exit 0");
    const found = run(`${withPlayer}:${dirname(uv)}:${SYSTEM_PATH}`);
    const foundText = found.stdout + found.stderr;
    assert.match(foundText, /pw-play[^\n]*(found|ok)|(found|ok)[^\n]*pw-play/i, foundText);
    assert.doesNotMatch(foundText, /pw-play[^\n]*(missing|not found)/i, foundText);
    const noUvPlayer = spawnSync("sh", ["-c", `PATH="${dirname(uv)}:${SYSTEM_PATH}" command -v pw-play`], { encoding: "utf8" }).stdout.trim();
    if (!noUvPlayer) {
      const missing = run(`${without}:${dirname(uv)}:${SYSTEM_PATH}`);
      const missingText = missing.stdout + missing.stderr;
      assert.match(missingText, /pw-play[^\n]*(missing|not found)|(missing|not found)[^\n]*pw-play/i, missingText);
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("voice mode is per session: a session that did not turn it on stays silent", async () => {
  const s = setup();
  try {
    s.state(true, "listening");
    const t = transcript(s.root, [["background work", REPLY]]);
    const p = s.run("prompt", { session_id: "headless", prompt: "background work", hook_event_name: "UserPromptSubmit" });
    assert.equal(p.stdout.trim(), "", "the rules reached a session without voice mode");
    assert.equal(s.run("stop", { session_id: "headless", transcript_path: t, hook_event_name: "Stop" }).status, 0);
    await sleep(500);
    assert.deepEqual(s.engineCalls(), [], "a session without voice mode spoke");
    assert.deepEqual(files(s.data).filter((f) => f.endsWith(".md")), [], "a session without voice mode wrote a transcript");
  } finally {
    s.cleanup();
  }
});

test("the stop hook speaks last_assistant_message when the transcript does not have the reply yet", async () => {
  const s = setup();
  try {
    s.state(true, "race");
    // Claude Code runs Stop before it writes the last reply: the file ends at the owner prompt.
    const t = join(s.root, "race.jsonl");
    writeFileSync(t, JSON.stringify({ type: "user", message: { role: "user", content: "are you there" } }) + "\n");
    s.run("prompt", { session_id: "race", prompt: "are you there", hook_event_name: "UserPromptSubmit" });
    const r = s.run("stop", { session_id: "race", transcript_path: t, last_assistant_message: "Yes, I am here.", hook_event_name: "Stop" });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(await until(() => s.engineCalls().some((c) => c.includes("Yes, I am here."))), `engine calls: ${JSON.stringify(s.engineCalls())}`);
    const md = files(s.data).filter((f) => f.endsWith(".md")).map((f) => readFileSync(f, "utf8")).join("");
    assert.match(md, /are you there/);
    assert.match(md, /Yes, I am here\./);
  } finally {
    s.cleanup();
  }
});

test("without a Wayland display the stop hook speaks but never starts the pill", async () => {
  const s = setup();
  try {
    s.state(true, "headless-display");
    const started = join(s.root, "qs-started");
    script(join(s.bin, "qs"), `touch "${started}"`);
    const t = transcript(s.root, [["go", REPLY]]);
    assert.equal(s.run("stop", { session_id: "headless-display", transcript_path: t, hook_event_name: "Stop" }).status, 0);
    assert.ok(await until(() => existsSync(s.plays)), "the player never played");
    assert.equal(existsSync(started), false, "qs started with no display: Quickshell aborts and dumps core");
  } finally {
    s.cleanup();
  }
});

test("a reply forced by another stop hook is saved but neither spoken nor cuts off the current audio", async () => {
  const s = setup();
  try {
    s.state(true, "forced");
    const t = transcript(s.root, [["go", REPLY]]);
    const r = s.run("stop", { session_id: "forced", transcript_path: t, last_assistant_message: "Journal updated.", stop_hook_active: true, hook_event_name: "Stop" });
    assert.equal(r.status, 0, r.stderr);
    await sleep(500);
    assert.deepEqual(s.engineCalls(), [], "a forced reply was spoken");
    const md = files(s.data).filter((f) => f.endsWith(".md")).map((f) => readFileSync(f, "utf8")).join("");
    assert.match(md, /Journal updated\./, "the forced reply is still in the transcript");
  } finally {
    s.cleanup();
  }
});

test("inside the AgentSI app (AGENTSI_TERMINAL set) both hooks do nothing: the core runs voice mode", () => {
  const s = setup();
  try {
    s.state(true, "in-app");
    const t = transcript(s.root, [["fix the build", REPLY]]);
    const env = { AGENTSI_TERMINAL: "t1" };
    const prompt = s.run("prompt", { session_id: "in-app", prompt: "fix the build" }, env);
    assert.equal(prompt.status, 0, prompt.stderr);
    assert.equal(prompt.stdout, "", "the prompt hook must add no rules");
    const stop = s.run("stop", { session_id: "in-app", transcript_path: t, last_assistant_message: REPLY }, env);
    assert.equal(stop.status, 0, stop.stderr);
    assert.equal(stop.stdout, "");
    assert.deepEqual(s.engineCalls(), [], "nothing is spoken");
    assert.deepEqual(files(s.data).filter((f) => f.endsWith(".md") || f.endsWith(".prompt.txt")), [], "no transcript, no saved prompt");
  } finally {
    s.cleanup();
  }
});
