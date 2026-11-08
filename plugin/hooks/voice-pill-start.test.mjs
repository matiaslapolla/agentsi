// S025: floating pill while voice mode speaks. The hook script is plugin/hooks/voice-mode.mjs (S024).
// Contract these tests pin (the story leaves it open, so the work must follow it):
//   - The pill is `qs -p <one .qml file>`, started by the Stop hook before any audio exists. The hook feeds it
//     JSON lines on its stdin: {"state":"preparing"}, then {"state":"playing","start":<epoch ms of the first word>,
//     "envelope":[levels, about 30 per second of audio]}, and {"state":"closed"} (qs also exits on stdin EOF).
//   - A click on the pill is a JSON line {"event":"stop"} on the stdout of qs.
//   - ONE `agentsi-voice say pm <dir>` process serves the whole reply: it reads one sentence per stdin line,
//     writes <dir>/<n>.wav for each, and prints `WAV <path>` on stdout for each.
//   - The chime is a file whose name contains "chime", played by the player before the first sentence;
//     the voice state setting {"chime": false} turns it off.
//   - Player is `pw-play`, found on PATH. Voice state: <AGENTSI_HOME>/voice/<session>.json, as in S024.
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

function lab({ seconds = 1, engineDelay = 600, player = "short" } = {}) {
  const root = mkdtempSync(join(tmpdir(), "voice-pill-"));
  const data = join(root, "data");
  const bin = join(root, "bin");
  const L = { qs: join(root, "qs.log"), engine: join(root, "engine.log"), plays: join(root, "plays.log"), qsPid: join(root, "qs.pid"), playerPid: join(root, "player.pid") };
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
setInterval(() => {}, 1000);`,
    true,
  );
  script(
    join(bin, "agentsi-voice"),
    `const fs = require("fs"), path = require("path");
const argv = process.argv.slice(2);
fs.appendFileSync(${JSON.stringify(L.engine)}, Date.now() + " START " + JSON.stringify(argv) + "\\n");
if (argv[0] !== "say") process.exit(0);
const dir = argv[argv.length - 1];
function wav(sec) {
  const rate = 16000, n = Math.floor(rate * sec), b = Buffer.alloc(44 + n * 2);
  b.write("RIFF", 0); b.writeUInt32LE(36 + n * 2, 4); b.write("WAVEfmt ", 8); b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28);
  b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write("data", 36); b.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) b.writeInt16LE(Math.round((Math.floor(i / (rate / 2)) % 2 ? 0.1 : 0.8) * 32767 * Math.sin(i / 20)), 44 + i * 2);
  return b;
}
let buf = "", k = 0, chain = Promise.resolve();
process.stdin.on("data", (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf("\\n")) >= 0) {
    const sentence = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!sentence) continue;
    const n = k++;
    chain = chain.then(async () => {
      fs.appendFileSync(${JSON.stringify(L.engine)}, Date.now() + " SENTENCE " + n + " " + sentence + "\\n");
      await new Promise((r) => setTimeout(r, ${engineDelay}));
      fs.mkdirSync(dir, { recursive: true });
      const p = path.join(dir, n + ".wav");
      fs.writeFileSync(p, wav(${seconds}));
      fs.appendFileSync(${JSON.stringify(L.engine)}, Date.now() + " AUDIO " + n + " " + p + "\\n");
      process.stdout.write("WAV " + p + "\\n");
    });
  }
});
process.stdin.on("end", () => chain.then(() => process.exit(0)));`,
    true,
  );
  const playBody = { long: "exec sleep 60", short: "sleep 0.3", fail: 'echo "player boom: no sound card" >&2\nexit 4' }[player];
  script(join(bin, "pw-play"), `echo "$(date +%s%3N) PLAY $*" >> "${L.plays}"\necho $$ > "${L.playerPid}"\n${playBody}`);
  const env = { PATH: `${bin}:${SYSTEM_PATH}`, AGENTSI_HOME: data, HOME: root, WAYLAND_DISPLAY: "wayland-test", NO_COLOR: "1" };
  return {
    root, L, env,
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
      return spawnSync(process.execPath, [HOOK, "stop"], { input: JSON.stringify({ session_id: session, transcript_path: path, hook_event_name: "Stop" }), encoding: "utf8", env, timeout: 15000 });
    },
    qsPid: () => Number(read(L.qsPid).trim()) || 0,
    cleanup() {
      for (const f of [L.qsPid, L.playerPid]) {
        const p = Number(read(f).trim());
        if (p && alive(p)) process.kill(p, "SIGKILL");
      }
      rmSync(root, { recursive: true, force: true });
    },
  };
}

const REPLY = "The build passes now. I fixed two tests. Nothing else broke.\n\nOn screen only\n\nDetails stay written.";

test("the pill opens in the preparing state before speech is ready, and one say process speaks the whole reply", async () => {
  const s = lab({ engineDelay: 700 });
  try {
    s.on("pill-1");
    const r = s.stop("pill-1", REPLY);
    assert.equal(r.status, 0, r.stderr);

    // The pill starts with one QML file.
    assert.ok(await until(() => lines(s.L.qs).some((l) => / START /.test(l))), "the pill (qs) was never started");
    const start = lines(s.L.qs).find((l) => / START /.test(l));
    const argv = JSON.parse(start.slice(start.indexOf(" START ") + 7));
    const qml = argv.filter((a) => a.endsWith(".qml"));
    assert.equal(qml.length, 1, `qs must get exactly one .qml file, got ${JSON.stringify(argv)}`);
    assert.ok(existsSync(qml[0]), `the QML file does not exist: ${qml[0]}`);
    assert.ok(read(qml[0]).trim().length > 0, "the QML file is empty");

    // It shows 'preparing' before the engine has produced any audio.
    assert.ok(await until(() => lines(s.L.engine).some((l) => / AUDIO /.test(l)), 15000), "the engine never produced audio");
    const firstAudio = Number(lines(s.L.engine).find((l) => / AUDIO /.test(l)).split(" ")[0]);
    const preparing = lines(s.L.qs).find((l) => / IN /.test(l) && JSON.parse(l.slice(l.indexOf(" IN ") + 4)).state === "preparing");
    assert.ok(preparing, `the pill never got the preparing state: ${read(s.L.qs)}`);
    assert.ok(Number(preparing.split(" ")[0]) < firstAudio, "the pill got 'preparing' only after the audio was ready");
    assert.ok(Number(start.split(" ")[0]) < firstAudio, "the pill started only after the audio was ready");

    // One say process serves every sentence.
    assert.ok(await until(() => lines(s.L.engine).filter((l) => / SENTENCE /.test(l)).length >= 3), `engine log: ${read(s.L.engine)}`);
    const says = lines(s.L.engine).filter((l) => / START /.test(l) && JSON.parse(l.slice(l.indexOf(" START ") + 7))[0] === "say");
    assert.equal(says.length, 1, `expected one say process, got ${says.length}: ${read(s.L.engine)}`);
    const spoken = lines(s.L.engine).filter((l) => / SENTENCE /.test(l)).join("\n");
    for (const part of ["The build passes now", "I fixed two tests", "Nothing else broke"]) assert.ok(spoken.includes(part), `sentence missing: ${part}`);
    assert.equal(spoken.includes("Details stay written"), false, "screen-only text was spoken");
  } finally {
    s.cleanup();
  }
});
