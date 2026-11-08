// S025: without qs on PATH voice still works with no pill and no error, and doctor names qs as missing.
// Contract: same as the other voice-pill tests. The pill is `qs`, the engine `agentsi-voice say pm <dir>`
// (one sentence per stdin line, one <dir>/<n>.wav and one `WAV <path>` stdout line each), the player `pw-play`.
// `agentsi-voice doctor` prints one line about qs, like the one it prints about the audio player:
// "... qs ...: found" or "... qs ...: missing".
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
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
const read = (p) => (existsSync(p) ? readFileSync(p, "utf8") : "");
const lines = (p) => read(p).split("\n").filter(Boolean);
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

test("without qs the reply still plays through the player, with no pill and no error", async () => {
  const root = mkdtempSync(join(tmpdir(), "voice-pill-"));
  try {
    const data = join(root, "data");
    const bin = join(root, "bin");
    const plays = join(root, "plays.log");
    mkdirSync(data);
    mkdirSync(bin);
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
    script(join(bin, "pw-play"), `echo "PLAY $*" >> "${plays}"\nexit 0`);
    // PATH holds only the fakes: there is no qs anywhere on it.
    const env = { PATH: bin, AGENTSI_HOME: data, HOME: root, WAYLAND_DISPLAY: "wayland-test", NO_COLOR: "1" };
    mkdirSync(join(data, "voice"), { recursive: true });
    writeFileSync(join(data, "voice", "noqs-1.json"), JSON.stringify({ code: REPO }));
    const t = join(root, "session.jsonl");
    writeFileSync(
      t,
      [
        JSON.stringify({ type: "user", message: { role: "user", content: "go" } }),
        JSON.stringify({ type: "assistant", message: { role: "assistant", content: [{ type: "text", text: "The build passes now. I fixed two tests.\n\nOn screen only\n\nDetails." }] } }),
      ].join("\n") + "\n",
    );
    const r = spawnSync(process.execPath, [HOOK, "stop"], {
      input: JSON.stringify({ session_id: "noqs-1", transcript_path: t, hook_event_name: "Stop" }),
      encoding: "utf8",
      env,
      timeout: 15000,
    });
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.stderr.trim(), "", "the hook printed an error without qs");
    if (r.stdout.trim()) assert.notEqual(JSON.parse(r.stdout).decision, "block", "the hook blocked");

    // The words still play, one file per sentence, through the player.
    assert.ok(await until(() => lines(plays).filter((l) => l.includes(".wav")).length >= 2, 15000), `the reply did not play: ${read(plays)}`);
    // No pill: nothing in the sandbox looks like a QML file or a pill process, and nothing was logged as an error.
    assert.deepEqual(files(root).filter((f) => f.endsWith(".qml") && f.startsWith(join(root, "bin"))), []);
    await sleep(500);
    const logs = files(data).filter((f) => f.endsWith(".log"));
    assert.deepEqual(logs.filter((f) => read(f).trim() !== ""), [], `an error was logged: ${logs.map(read).join("\n")}`);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("agentsi-voice doctor reports qs as missing when it is not on PATH", () => {
  const uv = spawnSync("sh", ["-c", "command -v uv"], { encoding: "utf8" }).stdout.trim();
  assert.ok(uv, "uv is needed to run agentsi-voice");
  const root = mkdtempSync(join(tmpdir(), "voice-doctor-"));
  try {
    // A PATH with uv and python but without qs (qs may sit in /usr/bin on this machine).
    const tools = join(root, "tools");
    mkdirSync(tools);
    for (const name of ["uv", "python3", "python"]) {
      const found = spawnSync("sh", ["-c", `PATH="${dirname(uv)}:${SYSTEM_PATH}" command -v ${name}`], { encoding: "utf8" }).stdout.trim();
      if (found) symlinkSync(found, join(tools, name));
    }
    const path = tools;
    assert.equal(existsSync(join(tools, "qs")), false);
    const out = spawnSync(uv, ["run", "--offline", "--project", join(REPO, "packages/voice"), "agentsi-voice", "doctor"], {
      encoding: "utf8",
      env: { PATH: path, HOME: process.env.HOME, NO_COLOR: "1" },
      timeout: 60000,
    });
    const text = out.stdout + out.stderr;
    assert.match(text, /\bqs\b[^\n]*(missing|not found)|(missing|not found)[^\n]*\bqs\b/i, text);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
