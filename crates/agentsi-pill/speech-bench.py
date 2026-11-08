#!/usr/bin/env python3
"""End-to-end timings of the speech of the pill, against a real core.

It starts its own core and pill on a private socket and data root, makes a
terminal with voice mode on, and for each run:

1. sends `hook.event` Stop with a reply of 2 sentences, and waits for the pill
   line "first sound at <ns>": the first sample of the reply in the output stream;
2. while the reply plays, sends `hook.event` UserPromptSubmit, which makes the
   core send `voice.stop`, and waits for the pill line "silence at <ns>": the first
   buffer of only zeros, or the pause of the stream. Sound already in the device
   buffer (20 ms asked) still plays after that;
3. with `--criterion`, reads `hyprctl activewindow -j` before the Stop hook, and
   `hyprctl layers -j` and `hyprctl activewindow -j` while the reply plays. The exit code is then the overlay criterion only.

The sound goes to a PipeWire null sink that the script loads and unloads, so
repeated runs make no sound. `--speakers` plays to the default output instead.
Both clocks are CLOCK_REALTIME of this machine. Usage:
  speech-bench.py [--runs 10] [--criterion] [--speakers] [--bin <target dir with agentsi-core and agentsi-pill>]
"""

import argparse
import json
import os
import queue
import re
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time

SINK = "agentsi_speech_bench"
REPLY = "The build passed, and the tests are green. I fixed the bug in the parser."


class Core:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(path)
        self.file = self.sock.makefile("rwb")
        self.next = 0
        self.call("core.hello", {"framed": False, "client": "speech-bench"})

    def call(self, method, params):
        self.next += 1
        line = json.dumps({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params})
        self.file.write(line.encode() + b"\n")
        self.file.flush()
        while True:
            reply = json.loads(self.file.readline())
            if reply.get("id") == self.next:
                if "error" in reply:
                    raise RuntimeError(f"{method}: {reply['error']}")
                return reply.get("result")


def hyprctl(*args):
    return json.loads(subprocess.run(["hyprctl", *args, "-j"], capture_output=True, text=True, check=True).stdout)


def overlay_layer():
    for output, v in hyprctl("layers").items():
        for level, layers in v["levels"].items():
            for layer in layers:
                if layer["namespace"] == "agentsi-pill" and layer["w"] > 0:
                    return f"{output} level {level}"
    return None


def wait_line(lines, pattern, timeout):
    end = time.time() + timeout
    while True:
        left = end - time.time()
        if left <= 0:
            return None
        try:
            line = lines.get(timeout=left)
        except queue.Empty:
            return None
        m = re.search(pattern, line)
        if m:
            return m


def null_sink():
    """Loads a null sink and returns its module id. pipewire-alsa sends a client with PIPEWIRE_NODE there."""
    out = subprocess.run(
        ["pactl", "load-module", "module-null-sink", f"sink_name={SINK}", f"sink_properties=node.description={SINK}"],
        capture_output=True, text=True, check=True,
    )
    return out.stdout.strip()


def sink_of_pill():
    """The sink names that the pill plays to now, from `pactl list sink-inputs`."""
    sinks = {s.split("\t")[0]: s.split("\t")[1] for s in subprocess.run(["pactl", "list", "short", "sinks"], capture_output=True, text=True).stdout.splitlines()}
    out = subprocess.run(["pactl", "-f", "json", "list", "sink-inputs"], capture_output=True, text=True).stdout
    return sorted({sinks.get(str(i.get("sink")), "?") for i in json.loads(out or "[]") if "agentsi-pill" in json.dumps(i.get("properties", {}))})


def loadavg():
    return float(open("/proc/loadavg").read().split()[0])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=10)
    ap.add_argument("--criterion", action="store_true")
    ap.add_argument("--speakers", action="store_true", help="play to the default output, not to a null sink")
    ap.add_argument("--bin", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../target/release"))
    args = ap.parse_args()

    tmp = tempfile.mkdtemp(prefix="agentsi-speech-bench-")
    home = os.path.join(tmp, "home")
    os.makedirs(home)
    env = dict(os.environ, AGENTSI_HOME=home, AGENTSI_CORE_SOCKET=os.path.join(tmp, "core.sock"), AGENTSI_PILL_STATS="1")
    module = None
    if not args.speakers:
        module = null_sink()
        env["PIPEWIRE_NODE"] = SINK
    core = subprocess.Popen([os.path.join(args.bin, "agentsi-core"), "serve"], env=env, stderr=subprocess.DEVNULL)
    pill = None
    try:
        for _ in range(100):
            if os.path.exists(env["AGENTSI_CORE_SOCKET"]):
                break
            time.sleep(0.05)
        pill = subprocess.Popen([os.path.join(args.bin, "agentsi-pill")], env=env, stderr=subprocess.PIPE, text=True)
        lines = queue.Queue()
        log = []

        def read():
            for line in pill.stderr:
                log.append(line)
                lines.put(line)

        threading.Thread(target=read, daemon=True).start()
        if not wait_line(lines, r"voice ready", 60):
            sys.exit("the pill voice did not start:\n" + "".join(log))
        c = Core(env["AGENTSI_CORE_SOCKET"])
        term = c.call("terminal.create", {"cwd": tmp, "argv": ["sleep", "3600"]})["id"]
        c.call("voice.mode", {"terminal": term, "on": True})
        time.sleep(0.5)

        def hook(event, payload):
            c.call("hook.event", {"terminal": term, "session": "bench", "event": event, "payload": payload})

        starts, stops, loads, checks = [], [], [], []
        for run in range(args.runs):
            hook("UserPromptSubmit", {"prompt": f"run {run}"})
            time.sleep(0.3)
            while not lines.empty():
                lines.get()
            load = loadavg()
            before = hyprctl("activewindow").get("address", "") if args.criterion else ""
            t0 = time.time_ns()
            hook("Stop", {"last_assistant_message": REPLY, "stop_hook_active": False})
            m = wait_line(lines, r"first sound at (\d+)", 10)
            if not m:
                sys.exit(f"run {run}: no sound:\n" + "".join(log[-20:]))
            starts.append((int(m.group(1)) - t0) / 1e6)
            time.sleep(0.6)
            if run == 0:
                print(f"the pill plays to: {', '.join(sink_of_pill()) or 'unknown'}", flush=True)
            if args.criterion:
                layer = overlay_layer()
                after = hyprctl("activewindow").get("address", "")
                checks.append((layer, before, after))
            t2 = time.time_ns()
            hook("UserPromptSubmit", {"prompt": f"stop {run}"})
            m = wait_line(lines, r"silence at (\d+)", 5)
            if not m:
                sys.exit(f"run {run}: no silence:\n" + "".join(log[-20:]))
            stops.append((int(m.group(1)) - t2) / 1e6)
            loads.append(max(load, loadavg()))
            print(f"run {run + 1}: first sound {starts[-1]:.0f} ms, stop {stops[-1]:.1f} ms, load {loads[-1]:.2f}", flush=True)
            time.sleep(1.0)

        quiet = max(loads) <= 4

        def summary(name, values, target):
            p50 = statistics.median(values)
            verdict = ("PASS" if max(values) <= target else "FAIL") if quiet else "NO VERDICT"
            print(f"{name}: p50 {p50:.1f} ms, max {max(values):.1f} ms, target {target} ms: {verdict}")
            return verdict

        verdicts = [
            summary("Stop hook to first sound", starts, 1500),
            summary("voice.stop to silence in the output stream", stops, 200),
        ]
        print(f"load average: min {min(loads):.2f}, max {max(loads):.2f}")
        if not quiet:
            print("NO VERDICT: the load average was above 4")
        for line in log:
            if "output " in line and "Hz" in line:
                print(line.strip())
        if args.criterion:
            ok = all(layer and "level 3" in layer and before == after for layer, before, after in checks)
            print(f"overlay while speaking: {checks[0][0]}, focus {checks[0][1]} -> {checks[0][2]}, {len(checks)} checks")
            print("criterion:", "PASS" if ok else "FAIL")
            # With --criterion the exit code is the overlay criterion only. The timings are printed.
            sys.exit(0 if ok else 1)
        sys.exit(0 if all(v == "PASS" for v in verdicts) else 1 if quiet else 3)
    finally:
        if pill:
            pill.terminate()
            pill.wait(5)
        core.terminate()
        core.wait(5)
        if module:
            subprocess.run(["pactl", "unload-module", module], check=False)


if __name__ == "__main__":
    main()
