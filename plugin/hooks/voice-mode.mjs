#!/usr/bin/env node
// Voice mode hooks (S024).
//   node voice-mode.mjs prompt   UserPromptSubmit: add the voice style rules, stop the audio still playing
//   node voice-mode.mjs stop     Stop: speak the prose of the reply, save the turn to a Markdown transcript
// State: <AgentSI data>/voice/<session id>.json ({"code": "<AgentSI checkout>"}), written by `agentsi voice on`.
// Voice mode is per session, so headless `claude -p` sessions (meetings, story sessions) never speak.
// Inside the app (AGENTSI_TERMINAL is set) it does nothing: the core runs voice mode.
// A hook never blocks the session: every failure is logged to <data>/voice/voice-mode.log and the exit code is 0.
import { spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCREEN_LINE = "On screen only";
const VOICE = "pm";
const RULES = [
  "Voice mode is on: the owner listens to your reply through a speaker and reads the rest on screen.",
  "Write the reply in two parts. First, a short spoken part in plain sentences: at most a few sentences, no Markdown, no lists, no tables, no code, no file paths, no URLs.",
  `Then, if there is anything that needs the eyes (code, commands, paths, tables, long lists, details), put a line with exactly "${SCREEN_LINE}" and write all of it below that line.`,
  `Everything above the "${SCREEN_LINE}" line is read aloud, so keep it short and say the result first.`,
  `If the whole reply is short and plain, you do not need the "${SCREEN_LINE}" line.`,
].join("\n");

/** Same lookup as the CLI (packages/core/src/paths.ts), minus the code checkout: this file runs from the plugin cache. */
function dataRoot() {
  const fromHome = (p) => resolve(homedir(), p.trim().replace(/^~(?=\/|$)/, homedir()));
  if (process.env.AGENTSI_HOME?.trim()) return fromHome(process.env.AGENTSI_HOME);
  try {
    const file = join(process.env.XDG_CONFIG_HOME?.trim() || join(homedir(), ".config"), "agentsi", "home");
    const line = readFileSync(file, "utf8").split("\n")[0].trim();
    return line ? fromHome(line) : null;
  } catch {
    return null;
  }
}

function dataDir() {
  const root = dataRoot();
  if (!root) throw new Error("no AgentSI data folder: set AGENTSI_HOME or ~/.config/agentsi/home");
  return join(root, "voice");
}

function logError(message) {
  try {
    const dir = dataDir();
    mkdirSync(dir, { recursive: true });
    appendFileSync(join(dir, "voice-mode.log"), `${new Date().toISOString()} ${String(message).trim()}\n`);
  } catch {
    // Nowhere left to report to.
  }
}

const safeId = (sessionId) => String(sessionId || "").replace(/[^\w-]/g, "_");

/** The state of a session with voice mode on, or null. */
function voiceState(sessionId) {
  if (!safeId(sessionId)) return null;
  try {
    return JSON.parse(readFileSync(join(dataDir(), `${safeId(sessionId)}.json`), "utf8"));
  } catch {
    return null;
  }
}

const onPath = (name) => (process.env.PATH ?? "").split(delimiter).some((d) => d && existsSync(join(d, name)));

/** `agentsi-voice` when it is installed on PATH, else the voice package of the checkout that turned voice mode on. */
function engineCommand(state) {
  if (onPath("agentsi-voice") || !state.code) return ["agentsi-voice"];
  return ["uv", "run", "--quiet", "--project", join(state.code, "packages/voice"), "agentsi-voice"];
}

function readPayload() {
  try {
    return JSON.parse(readFileSync(0, "utf8") || "{}");
  } catch {
    return {};
  }
}

const speakerPidFile = (sessionId) => join(dataDir(), `${safeId(sessionId)}.speaker.pid`);

/** Stops the speaker of the previous reply of this session. The worker closes the pill; its process group is the fallback. */
function stopAudio(sessionId) {
  try {
    const pid = Number(readFileSync(speakerPidFile(sessionId), "utf8").trim());
    rmSync(speakerPidFile(sessionId), { force: true });
    if (pid <= 0) return;
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      return;
    }
    const alive = () => {
      try {
        process.kill(pid, 0);
        return true;
      } catch {
        return false;
      }
    };
    const end = Date.now() + 2500;
    const gate = new Int32Array(new SharedArrayBuffer(4));
    while (alive() && Date.now() < end) Atomics.wait(gate, 0, 0, 30);
    if (alive()) process.kill(-pid, "SIGKILL");
  } catch {
    // No speaker running.
  }
}

/** Markdown reply to the plain sentences to speak: only what is above the "On screen only" line. */
function spokenSentences(reply) {
  const lines = reply.split("\n");
  const cut = lines.findIndex((l) => l.replace(/[*_#>\s:]/g, "").toLowerCase() === SCREEN_LINE.replace(/\s/g, "").toLowerCase());
  let text = (cut >= 0 ? lines.slice(0, cut) : lines).join("\n");
  text = text
    .replace(/```[\s\S]*?(```|$)/g, "\n")
    .replace(/~~~[\s\S]*?(~~~|$)/g, "\n")
    .split("\n")
    .filter((l) => !/^\s{0,3}#{1,6}\s/.test(l) && !/^\s*\|/.test(l) && !/^\s*[-*_]{3,}\s*$/.test(l))
    .map((l) => l.replace(/^\s*(>|[-*+]|\d+[.)])\s+/, ""))
    .join("\n");
  text = text
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/`[^`]*`/g, " ")
    .replace(/https?:\/\/\S+/g, " ")
    .replace(/\S*\/\S*/g, " ")
    .replace(/\b[\w-]+\.(?:tsx?|jsx?|mjs|cjs|json|md|mdx|py|sh|toml|ya?ml|sql|css|html|lock|log|txt|env)\b/g, " ")
    .replace(/(\*\*|__|\*|_|~~)/g, "");
  return text
    .split(/\n\s*\n/)
    .map((p) => p.replace(/\s+/g, " ").trim())
    .filter(Boolean)
    .flatMap((p) => p.split(/(?<=[.!?])\s+/))
    .map((s) => s.replace(/\s+([,.;:!?])/g, "$1").trim())
    .filter((s) => /[\p{L}\p{N}]/u.test(s));
}

/** Last owner prompt and the full assistant text that followed it, from a Claude Code transcript (JSONL). */
function lastTurn(path) {
  let prompt = "";
  let replies = [];
  for (const line of readFileSync(path, "utf8").split("\n")) {
    if (!line.trim()) continue;
    let entry;
    try {
      entry = JSON.parse(line);
    } catch {
      continue;
    }
    const message = entry.message;
    if (!message) continue;
    const content = message.content;
    // Only real turns: Claude Code also logs `api-request-blob` entries with a user role.
    if (entry.type === "user") {
      const text = typeof content === "string" ? content : (content ?? []).filter((b) => b.type === "text").map((b) => b.text).join("\n");
      if (text.trim()) {
        prompt = text.trim();
        replies = [];
      }
    } else if (entry.type === "assistant") {
      const text = typeof content === "string" ? content : (content ?? []).filter((b) => b.type === "text").map((b) => b.text).join("\n");
      if (text.trim()) replies.push(text.trim());
    }
  }
  return { prompt, reply: replies.join("\n\n") };
}

function saveTranscript(sessionId, prompt, reply, sentences) {
  const dir = join(dataDir(), "transcripts");
  mkdirSync(dir, { recursive: true });
  const file = join(dir, `${String(sessionId || "session").replace(/[^\w.-]/g, "_")}.md`);
  const head = existsSync(file) ? "" : `# Voice session ${sessionId || ""}\n\n`;
  const spoken = sentences.length ? sentences.map((s) => `> ${s}`).join("\n") : "> (nothing spoken)";
  appendFileSync(
    file,
    `${head}## ${new Date().toISOString()}\n\n**Owner:**\n\n${prompt}\n\n**Reply:**\n\n${reply}\n\n**Spoken part (voice ${VOICE}):**\n\n${spoken}\n\n`,
  );
}

const promptFile = (sessionId) => join(dataDir(), `${safeId(sessionId)}.prompt.txt`);

function runPrompt() {
  const payload = readPayload();
  if (!voiceState(payload.session_id)) return;
  stopAudio(payload.session_id);
  try {
    if (typeof payload.prompt === "string") writeFileSync(promptFile(payload.session_id), payload.prompt);
  } catch (e) {
    logError(`prompt save: ${e.message}`);
  }
  process.stdout.write(JSON.stringify({ hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: RULES } }));
}

function runStop() {
  const payload = readPayload();
  const state = voiceState(payload.session_id);
  if (!state) return;
  // Claude Code runs Stop before it writes the last reply to the transcript file, so the payload is the source.
  let turn = { prompt: "", reply: "" };
  try {
    turn = lastTurn(payload.transcript_path);
  } catch (e) {
    if (!payload.last_assistant_message) {
      logError(`transcript: ${e.message}`);
      return;
    }
  }
  if (typeof payload.last_assistant_message === "string" && payload.last_assistant_message.trim()) turn.reply = payload.last_assistant_message.trim();
  try {
    turn.prompt = readFileSync(promptFile(payload.session_id), "utf8").trim() || turn.prompt;
  } catch {
    // No prompt saved: keep the one from the transcript.
  }
  if (!turn.reply) return;
  const sentences = spokenSentences(turn.reply);
  try {
    saveTranscript(payload.session_id, turn.prompt, turn.reply, sentences);
  } catch (e) {
    logError(`transcript write: ${e.message}`);
  }
  // A reply that another Stop hook forced (such as a journal update) must not cut off the reply the owner is hearing.
  if (!sentences.length || payload.stop_hook_active) return;
  stopAudio(payload.session_id);
  const work = join(dataDir(), "tmp");
  mkdirSync(work, { recursive: true });
  const dir = mkdtempSync(join(work, "job-"));
  const job = join(dir, "job.json");
  const pidFile = speakerPidFile(payload.session_id);
  writeFileSync(job, JSON.stringify({ sentences, dir, engine: engineCommand(state), pidFile, chime: state.chime }));
  const child = spawn(process.execPath, [fileURLToPath(import.meta.url), "speak", job], { detached: true, stdio: "ignore", env: process.env });
  child.unref();
  try {
    writeFileSync(pidFile, String(child.pid));
  } catch (e) {
    logError(`speaker pid: ${e.message}`);
  }
}

const hookDir = dirname(fileURLToPath(import.meta.url));
const PILL_QML = join(hookDir, "voice-pill.qml");
const ENVELOPE_PER_SECOND = 30;
const CHIME_SECONDS = 0.35;

/** A short two-note chime as a 16-bit mono wav, written to `file`. */
function writeChime(file) {
  const rate = 22050;
  const n = Math.floor(rate * 0.35);
  const b = Buffer.alloc(44 + n * 2);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + n * 2, 4);
  b.write("WAVEfmt ", 8);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate * 2, 28);
  b.writeUInt16LE(2, 32);
  b.writeUInt16LE(16, 34);
  b.write("data", 36);
  b.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) {
    const t = i / rate;
    const local = t % 0.175;
    const gain = Math.exp(-6 * local) * Math.min(1, local * 400);
    b.writeInt16LE(Math.round(0.3 * 32767 * gain * Math.sin(2 * Math.PI * (t < 0.175 ? 880 : 1320) * t)), 44 + i * 2);
  }
  writeFileSync(file, b);
}

/** Level envelope of a 16-bit PCM wav file: the RMS of each 1/30 s window, from 0 to 1. */
function envelopeOf(file) {
  const buf = readFileSync(file);
  let pos = 12;
  let channels = 1;
  let rate = 16000;
  while (pos + 8 <= buf.length) {
    const id = buf.toString("ascii", pos, pos + 4);
    const size = buf.readUInt32LE(pos + 4);
    if (id === "fmt ") {
      channels = Math.max(1, buf.readUInt16LE(pos + 10));
      rate = buf.readUInt32LE(pos + 12);
    } else if (id === "data") {
      const end = Math.min(buf.length, pos + 8 + size);
      const frames = Math.floor((end - pos - 8) / (2 * channels));
      const window = Math.max(1, Math.round(rate / ENVELOPE_PER_SECOND));
      const out = [];
      for (let f = 0; f < frames; f += window) {
        const last = Math.min(frames, f + window);
        let sum = 0;
        for (let i = f; i < last; i++) {
          const v = buf.readInt16LE(pos + 8 + i * 2 * channels) / 32768;
          sum += v * v;
        }
        out.push(Math.min(1, Math.round(Math.sqrt(sum / (last - f)) * 1000) / 1000));
      }
      return out;
    }
    pos += 8 + size + (size % 2);
  }
  return [];
}

/** Background worker: the pill, one engine process for all sentences, and the player, one file at a time. */
async function runSpeak(jobPath) {
  const { sentences, dir, engine, pidFile, chime } = JSON.parse(readFileSync(jobPath, "utf8"));
  const [cmd, ...pre] = engine;
  let stopped = false;
  let player = null;
  let engineProc = null;
  let pill = null;
  let pillExit = null;
  let waiter = null;
  const wavs = [];

  const wake = () => {
    const w = waiter;
    waiter = null;
    w?.();
  };
  const send = (msg) => {
    try {
      if (pill && pill.stdin.writable) pill.stdin.write(`${JSON.stringify(msg)}\n`);
    } catch {
      // The pill is gone.
    }
  };
  const haltAudio = () => {
    stopped = true;
    player?.kill("SIGKILL");
    engineProc?.kill("SIGKILL");
    wake();
  };
  async function closePill() {
    if (!pill) return;
    const open = pill;
    send({ state: "closed" });
    try {
      open.stdin.end();
    } catch {
      // Already closed.
    }
    const timer = setTimeout(() => open.kill("SIGKILL"), 2000);
    await pillExit;
    clearTimeout(timer);
    pill = null;
  }

  /** Plays one file; a stop counts as ok. */
  function play(file) {
    return new Promise((done) => {
      let stderr = "";
      let child;
      try {
        child = spawn("pw-play", [file], { stdio: ["ignore", "ignore", "pipe"] });
      } catch (e) {
        done({ ok: false, error: `pw-play: ${e.message}` });
        return;
      }
      player = child;
      child.stderr.on("data", (d) => (stderr += d));
      child.on("error", (e) => done({ ok: false, error: `pw-play: ${e.message}` }));
      child.on("close", (code) => done({ ok: code === 0 || stopped, error: `pw-play exited ${code}: ${stderr}` }));
    });
  }

  process.on("SIGTERM", haltAudio);

  try {
    // Without a Wayland display (tests, ssh, headless runs) Quickshell aborts at start: show no pill.
    if (process.env.WAYLAND_DISPLAY && onPath("qs") && existsSync(PILL_QML)) {
      try {
        // The pill reads /proc/<qs>/fd/0 and writes /proc/<qs>/fd/1. Node gives a child sockets, which /proc cannot
        // open, so `cat` on each side turns them into plain pipes.
        const child = spawn("sh", ["-c", 'cat | qs -p "$1" | cat', "voice-pill", PILL_QML], { stdio: ["pipe", "pipe", "ignore"] });
        pill = child;
        pillExit = new Promise((done) => {
          child.on("close", done);
          child.on("error", () => {
            pill = null;
            done();
          });
        });
        child.stdin.on("error", () => {});
        let out = "";
        child.stdout.on("data", (d) => {
          out += d;
          let i;
          while ((i = out.indexOf("\n")) >= 0) {
            const line = out.slice(0, i);
            out = out.slice(i + 1);
            try {
              if (JSON.parse(line).event === "stop") haltAudio();
            } catch {
              // Not a pill event.
            }
          }
        });
        send({ state: "preparing" });
      } catch (e) {
        logError(`pill: ${e.message}`);
        pill = null;
      }
    }

    let engineError = "";
    let engineCode = 0;
    let finished = false;
    try {
      const proc = spawn(cmd, [...pre, "say", VOICE, dir], { stdio: ["pipe", "pipe", "pipe"] });
      engineProc = proc;
      let out = "";
      proc.stdout.on("data", (d) => {
        out += d;
        let i;
        while ((i = out.indexOf("\n")) >= 0) {
          const line = out.slice(0, i).trim();
          out = out.slice(i + 1);
          if (line.startsWith("WAV ")) {
            wavs.push(line.slice(4));
            wake();
          }
        }
      });
      proc.stderr.on("data", (d) => (engineError += d));
      proc.on("error", (e) => {
        engineError += `${cmd}: ${e.message}`;
        engineCode = 1;
        finished = true;
        wake();
      });
      proc.on("close", (code) => {
        engineCode = code ?? 1;
        finished = true;
        wake();
      });
      proc.stdin.on("error", () => {});
      proc.stdin.end(`${sentences.join("\n")}\n`);
    } catch (e) {
      engineError = `${cmd}: ${e.message}`;
      engineCode = 1;
      finished = true;
    }

    let first = true;
    let spoken = 0;
    while (!stopped) {
      while (!wavs.length && !finished && !stopped) await new Promise((done) => (waiter = done));
      if (stopped || !wavs.length) break;
      const file = wavs.shift();
      let envelope = [];
      try {
        envelope = envelopeOf(file);
      } catch (e) {
        logError(`envelope: ${e.message}`);
      }
      const lead = first && chime !== false ? CHIME_SECONDS * 1000 : 0;
      // The pill hears about the audio before the chime starts; "start" is when the first word begins.
      send({ state: "playing", start: Date.now() + lead, envelope });
      if (first) {
        first = false;
        if (chime !== false) {
          const chimeFile = join(dir, "chime.wav");
          writeChime(chimeFile);
          const c = await play(chimeFile);
          if (!c.ok) logError(`player: ${c.error}`);
          if (stopped || !c.ok) break;
        }
      }
      const played = await play(file);
      spoken++;
      if (!played.ok) {
        logError(`player: ${played.error}`);
        break;
      }
    }
    if (!stopped && engineCode !== 0 && spoken < sentences.length) logError(`engine: exited ${engineCode}: ${engineError}`);
  } catch (e) {
    logError(`speaker: ${e.message}`);
  } finally {
    haltAudio();
    await closePill();
    rmSync(dir, { recursive: true, force: true });
    // A stale pid can be reused by another process, which the next prompt would then kill.
    try {
      if (readFileSync(pidFile, "utf8").trim() === String(process.pid)) rmSync(pidFile, { force: true });
    } catch {
      // Already removed by a newer reply.
    }
  }
}

async function main() {
  const [event, arg] = process.argv.slice(2);
  // Inside the AgentSI app the core runs voice mode through `agentsi-core hook`. Two speakers would say a reply twice.
  if (process.env.AGENTSI_TERMINAL && (event === "prompt" || event === "stop")) process.exit(0);
  try {
    if (event === "prompt") runPrompt();
    else if (event === "stop") runStop();
    else if (event === "speak") await runSpeak(arg);
  } catch (e) {
    logError(`${event}: ${e.stack ?? e.message}`);
  }
  process.exit(0);
}

await main();
