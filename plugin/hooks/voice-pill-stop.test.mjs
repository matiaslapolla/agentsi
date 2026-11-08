// S025: a click on the pill, a new prompt, the end of audio and a failure each stop the player and close the pill.
// Contract these tests pin (the story leaves it open, so the work must follow it):
//   - The pill is `qs -p <one .qml file>`. The hook feeds it JSON lines on its stdin: {"state":"preparing"},
//     {"state":"playing","start":...,"envelope":[...]}, {"state":"closed"}. qs exits on "closed" or on stdin EOF.
//   - A click on the pill is a JSON line {"event":"stop"} on the stdout of qs. The fake qs prints it when the
//     test creates the file CLICK in the sandbox root.
//   - ONE `agentsi-voice say pm <dir>` process reads one sentence per stdin line, writes <dir>/<n>.wav for each
//     and prints `WAV <path>` on stdout for each.
//   - The player is `pw-play`. Voice state: <AGENTSI_HOME>/voice/<session>.json, as in S024.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";

const HOOK = new URL("./voice-mode.mjs", import.meta.url).pathname;
const REPO = resolve(new URL("../..", import.meta.url).pathname);
const SYSTEM_PATH = "/usr/bin:/bin";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(fn, ms = 8000) {
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
const read = (p) => (existsSync(p) ? readFileSync(p, "utf8") : "");
const lines = (p) => read(p).split("\n").filter(Boolean);
function script(path, body, node = false) {
  writeFileSync(path, node ? `#!${process.execPath}\n${body}\n` : `#!/bin/sh\n${body}\n`);
  chmodSync(path, 0o755);
}

// player: "long" (plays until killed), "short" (ends by itself), "fail" (exits 4)
function lab({ player }) {
  const root = mkdtempSync(join(tmpdir(), "voice-pill-"));
  const data = join(root, "data");
  const bin = join(root, "bin");
  const L = { qs: join(root, "qs.log"), plays: join(root, "plays.log"), qsPid: join(root, "qs.pid"), playerPid: join(root, "player.pid"), click: join(root, "CLICK") };
  mkdirSync(data);
  mkdirSync(bin);
  script(
    join(bin, "qs"),
    `const fs = require("fs");
fs.appendFileSync(${JSON.stringify(L.qs)}, Date.now() + " START " + JSON.stringify(process.argv.slice(2)) + "\\n");
fs.writeFileSync(${JSON.stringify(L.qsPid)}, String(process.pid));
let buf = "";
process.stdin.on("data", (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf("\\n")) >= 0) {
    const line = buf.slice(0, i);
    buf = buf.slice(i + 1);
    fs.appendFileSync(${JSON.stringify(L.qs)}, Date.now() + " IN " + line + "\\n");
    try { if (JSON.parse(line).state === "closed") process.exit(0); } catch {}
  }
});
process.stdin.on("end", () => process.exit(0));
setInterval(() => {
  if (fs.existsSync(${JSON.stringify(L.click)})) { fs.rmSync(${JSON.stringify(L.click)}); process.stdout.write('{"event":"stop"}\\n'); }
}, 50);`,
    true,
  );
  script(
    join(bin, "agentsi-voice"),
    `const fs = require("fs"), path = require("path");
const argv = process.argv.slice(2);
if (argv[0] !== "say") process.exit(0);
const dir = argv[argv.length - 1];
function wav(sec) {
  const rate = 16000, n = Math.floor(rate * sec), b = Buffer.alloc(44 + n * 2);
  b.write("RIFF", 0); b.writeUInt32LE(36 + n * 2, 4); b.write("WAVEfmt ", 8); b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28);
  b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write("data", 36); b.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) b.writeInt16LE(Math.round(0.5 * 32767 * Math.sin(i / 20)), 44 + i * 2);
  return b;
}
let buf = "", k = 0;
process.stdin.on("data", (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf("\\n")) >= 0) {
    const sentence = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!sentence) continue;
    fs.mkdirSync(dir, { recursive: true });
    const p = path.join(dir, k++ + ".wav");
    fs.writeFileSync(p, wav(1));
    process.stdout.write("WAV " + p + "\\n");
  }
});
process.stdin.on("end", () => process.exit(0));`,
    true,
  );
  const playBody = { long: "exec sleep 60", short: "sleep 0.3", fail: 'echo "player boom: no sound card" >&2\nexit 4' }[player];
  script(join(bin, "pw-play"), `echo "$(date +%s%3N) PLAY $*" >> "${L.plays}"\necho $$ > "${L.playerPid}"\n${playBody}`);
  const env = { PATH: `${bin}:${SYSTEM_PATH}`, AGENTSI_HOME: data, HOME: root, WAYLAND_DISPLAY: "wayland-test", NO_COLOR: "1" };
  const run = (event, payload) => spawnSync(process.execPath, [HOOK, event], { input: JSON.stringify(payload), encoding: "utf8", env, timeout: 15000 });
  return {
    root, L, run,
    on(session) {
      mkdirSync(join(data, "voice"), { recursive: true });
      writeFileSync(join(data, "voice", `${session}.json`), JSON.stringify({ code: REPO }));
    },
    stop(session, reply) {
      const path = join(root, "session.jsonl");
      writeFileSync(
        path,
        [JSON.stringify({ type: "user", message: { role: "user", content: "go" } }), JSON.stringify({ type: "assistant", message: { role: "assistant", content: [{ type: "text", text: reply }] } })].join("\n") + "\n",
      );
      return run("stop", { session_id: session, transcript_path: path, hook_event_name: "Stop" });
    },
    qsIn: () => lines(L.qs).filter((l) => / IN /.test(l)).map((l) => JSON.parse(l.slice(l.indexOf(" IN ") + 4))),
    qsPid: () => Number(read(L.qsPid).trim()) || 0,
    playerPid: () => Number(read(L.playerPid).trim()) || 0,
    cleanup() {
      for (const p of [Number(read(L.qsPid).trim()), Number(read(L.playerPid).trim())]) if (p && alive(p)) process.kill(p, "SIGKILL");
      rmSync(root, { recursive: true, force: true });
    },
  };
}

const REPLY = "The build passes now. I fixed two tests.\n\nOn screen only\n\nDetails stay written.";

// Audio is playing and the pill is open.
async function playing(s, session) {
  s.on(session);
  const r = s.stop(session, REPLY);
  assert.equal(r.status, 0, r.stderr);
  assert.ok(await until(() => s.qsIn().some((m) => m.state === "playing"), 15000), `the pill never got a playing state: ${read(s.L.qs)}`);
  assert.ok(await until(() => s.playerPid() > 0), "the player never started");
  assert.ok(alive(s.qsPid()), "the pill is not open while audio plays");
}

test("a click on the pill stops the player and closes the pill", async () => {
  const s = lab({ player: "long" });
  try {
    await playing(s, "click-1");
    const player = s.playerPid();
    assert.ok(alive(player), "the player should be playing before the click");
    writeFileSync(s.L.click, "");
    assert.ok(await until(() => !alive(player)), "the click did not stop the player");
    assert.ok(await until(() => !alive(s.qsPid())), "the click did not close the pill");
    assert.ok(s.qsIn().some((m) => m.state === "closed"), "the pill was killed instead of told to close");
  } finally {
    s.cleanup();
  }
});

test("a new prompt stops the player and closes the pill", async () => {
  const s = lab({ player: "long" });
  try {
    await playing(s, "prompt-1");
    const player = s.playerPid();
    const r = s.run("prompt", { session_id: "prompt-1", prompt: "next", hook_event_name: "UserPromptSubmit" });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(await until(() => !alive(player)), "the new prompt did not stop the player");
    assert.ok(await until(() => !alive(s.qsPid())), "the new prompt did not close the pill");
  } finally {
    s.cleanup();
  }
});

test("the pill closes when the audio ends", async () => {
  const s = lab({ player: "short" });
  try {
    s.on("end-1");
    assert.equal(s.stop("end-1", REPLY).status, 0);
    assert.ok(await until(() => s.qsIn().some((m) => m.state === "playing"), 15000), "the pill never got a playing state");
    assert.ok(await until(() => lines(s.L.plays).length >= 2, 15000), "both sentences should have played");
    assert.ok(await until(() => !alive(s.qsPid()), 10000), "the pill stayed open after the audio ended");
    assert.ok(s.qsIn().some((m) => m.state === "closed"), "the pill was not told to close");
  } finally {
    s.cleanup();
  }
});

test("the pill closes when the audio fails, and the hook never blocks", async () => {
  const s = lab({ player: "fail" });
  try {
    s.on("fail-1");
    const r = s.stop("fail-1", REPLY);
    assert.equal(r.status, 0, r.stderr);
    if (r.stdout.trim()) assert.notEqual(JSON.parse(r.stdout).decision, "block", "the hook blocked");
    assert.ok(await until(() => s.qsPid() > 0, 15000), "the pill never opened");
    assert.ok(await until(() => lines(s.L.plays).length >= 1, 15000), "the player never ran");
    assert.ok(await until(() => !alive(s.qsPid()), 10000), "the pill stayed open after the audio failed");
    assert.ok(s.qsIn().some((m) => m.state === "closed"), "the pill was not told to close");
  } finally {
    s.cleanup();
  }
});
