// S025: the pill gets the level envelope and the start time; the chime plays before the first word unless it is off.
// Contract these tests pin (the story leaves it open, so the work must follow it):
//   - The pill is `qs -p <one .qml file>`. The hook feeds it JSON lines on its stdin: {"state":"preparing"},
//     {"state":"playing","start":<epoch ms of the first word>,"envelope":[levels, about 30 per second of audio]},
//     {"state":"closed"}.
//   - ONE `agentsi-voice say pm <dir>` process reads one sentence per stdin line, writes <dir>/<n>.wav for each
//     and prints `WAV <path>` on stdout for each.
//   - The chime is a file whose name contains "chime", played by the player (`pw-play`) before the first sentence;
//     the voice state setting {"chime": false} in <AGENTSI_HOME>/voice/<session>.json turns it off.
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

function lab({ seconds = 2, engineDelay = 300, chime } = {}) {
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
if (argv[0] !== "say") process.exit(0);
const dir = argv[argv.length - 1];
function wav(sec) {
  const rate = 16000, n = Math.floor(rate * sec), b = Buffer.alloc(44 + n * 2);
  b.write("RIFF", 0); b.writeUInt32LE(36 + n * 2, 4); b.write("WAVEfmt ", 8); b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28);
  b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write("data", 36); b.writeUInt32LE(n * 2, 40);
  // Loud for the first half second, quiet for the next, and so on: the envelope must move with the audio.
  for (let i = 0; i < n; i++) b.writeInt16LE(Math.round((Math.floor(i / (rate / 2)) % 2 ? 0.05 : 0.8) * 32767 * Math.sin(i / 20)), 44 + i * 2);
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
      await new Promise((r) => setTimeout(r, ${engineDelay}));
      fs.mkdirSync(dir, { recursive: true });
      const p = path.join(dir, n + ".wav");
      fs.writeFileSync(p, wav(${seconds}));
      process.stdout.write("WAV " + p + "\\n");
    });
  }
});
process.stdin.on("end", () => chain.then(() => process.exit(0)));`,
    true,
  );
  script(join(bin, "pw-play"), `echo "$(date +%s%3N) PLAY $*" >> "${L.plays}"\necho $$ > "${L.playerPid}"\nsleep 0.3`);
  const env = { PATH: `${bin}:${SYSTEM_PATH}`, AGENTSI_HOME: data, HOME: root, WAYLAND_DISPLAY: "wayland-test", NO_COLOR: "1" };
  return {
    root, L, env,
    on(session) {
      mkdirSync(join(data, "voice"), { recursive: true });
      writeFileSync(join(data, "voice", `${session}.json`), JSON.stringify({ code: REPO, ...(chime === undefined ? {} : { chime }) }));
    },
    stop(session, reply) {
      const path = join(root, "session.jsonl");
      writeFileSync(
        path,
        [JSON.stringify({ type: "user", message: { role: "user", content: "go" } }), JSON.stringify({ type: "assistant", message: { role: "assistant", content: [{ type: "text", text: reply }] } })].join("\n") + "\n",
      );
      return spawnSync(process.execPath, [HOOK, "stop"], { input: JSON.stringify({ session_id: session, transcript_path: path, hook_event_name: "Stop" }), encoding: "utf8", env, timeout: 15000 });
    },
    qsIn: () => lines(L.qs).filter((l) => / IN /.test(l)).map((l) => ({ ts: Number(l.split(" ")[0]), msg: JSON.parse(l.slice(l.indexOf(" IN ") + 4)) })),
    plays: () => lines(L.plays).map((l) => ({ ts: Number(l.split(" ")[0]), args: l.slice(l.indexOf("PLAY ") + 5) })),
    cleanup() {
      for (const f of [L.qsPid, L.playerPid]) {
        const p = Number(read(f).trim());
        if (p && alive(p)) process.kill(p, "SIGKILL");
      }
      rmSync(root, { recursive: true, force: true });
    },
  };
}

// One spoken sentence, so the audio is exactly `seconds` long.
const REPLY = "The build passes now.\n\nOn screen only\n\nDetails stay written.";

async function speak(s, session) {
  s.on(session);
  const r = s.stop(session, REPLY);
  assert.equal(r.status, 0, r.stderr);
  assert.ok(await until(() => s.qsIn().some((m) => m.msg.state === "playing"), 15000), `the pill never got a playing state: ${read(s.L.qs)}`);
  // The chime is a .wav too: wait for the first word.
  assert.ok(await until(() => s.plays().some((p) => p.args.includes(".wav") && !/chime/i.test(p.args)), 15000), `the word audio never played: ${read(s.L.plays)}`);
}

test("the pill gets the envelope (about 30 values a second) and the start time", async () => {
  const s = lab({ seconds: 2 });
  try {
    await speak(s, "level-1");
    const playing = s.qsIn().filter((m) => m.msg.state === "playing" && Array.isArray(m.msg.envelope));
    assert.ok(playing.length >= 1, `no playing message with an envelope: ${read(s.L.qs)}`);
    const values = playing.flatMap((m) => m.msg.envelope);
    assert.ok(values.every((v) => typeof v === "number" && Number.isFinite(v)), "the envelope must hold numbers");
    // 2 seconds of audio at about 30 values a second.
    assert.ok(values.length >= 50 && values.length <= 70, `expected about 60 values for 2 s, got ${values.length}`);
    assert.ok(Math.max(...values) > Math.min(...values) * 2 + 1e-6, "the envelope is flat: it does not move with the audio");
    // The start time is the epoch ms of the first word.
    const start = playing[0].msg.start;
    assert.equal(typeof start, "number", "no start time");
    const wavPlay = s.plays().find((p) => p.args.includes(".wav"));
    assert.ok(Math.abs(start - wavPlay.ts) < 3000, `start ${start} is not close to the first word at ${wavPlay.ts}`);
    // The pill is told before the audio ends, not after.
    assert.ok(playing[0].ts <= wavPlay.ts + 1500, "the pill got the envelope long after playback started");
  } finally {
    s.cleanup();
  }
});

test("the chime plays before the first word by default", async () => {
  const s = lab({});
  try {
    await speak(s, "chime-on");
    const plays = s.plays();
    const chime = plays.findIndex((p) => /chime/i.test(p.args));
    const word = plays.findIndex((p) => p.args.includes(".wav") && !/chime/i.test(p.args));
    assert.ok(chime >= 0, `no chime was played: ${JSON.stringify(plays)}`);
    assert.ok(word >= 0, "no word audio was played");
    assert.ok(chime < word, "the chime must play before the first word");
  } finally {
    s.cleanup();
  }
});

test("the chime is skipped when the chime setting is off, and the words still play", async () => {
  const s = lab({ chime: false });
  try {
    await speak(s, "chime-off");
    await sleep(500);
    const plays = s.plays();
    assert.equal(plays.some((p) => /chime/i.test(p.args)), false, `a chime played with the setting off: ${JSON.stringify(plays)}`);
    assert.ok(plays.some((p) => p.args.includes(".wav")), "the words did not play");
    assert.ok(s.qsIn().some((m) => m.msg.state === "playing"), "the pill got no playing state");
  } finally {
    s.cleanup();
  }
});
