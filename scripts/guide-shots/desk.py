"""Drives the core of the guide shots for the desk conversation and the agent cards.

  desk.py <socket> desk <product>                 one owner question, a streamed reply and a few views
  desk.py <socket> speak <host> <role> <text...>  a voice.speak of a role, through voice.relay
"""
import json
import socket
import sys
import time

sock, action = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX)
s.connect(sock)
f = s.makefile("rw", buffering=1)
n = 0


def call(method, params):
    global n
    n += 1
    f.write(json.dumps({"jsonrpc": "2.0", "id": n, "method": method, "params": params}) + "\n")
    f.flush()
    while True:
        line = f.readline()
        if not line:
            raise SystemExit("desk.py: the core closed the socket")
        m = json.loads(line)
        if m.get("id") == n:
            if "error" in m:
                raise SystemExit(f"desk.py: {method}: {m['error']}")
            return m["result"]


if action == "desk":
    product = sys.argv[3]
    call("desk.send", {"product": product, "text": "What waits for me this week, and how are we doing?"})
    time.sleep(4)  # the stub streams for about 1.5 s
    session = call("desk.history", {"product": product})["session"]
    views = [
        {"kind": "decisions", "ids": [1]},
        {"kind": "table", "title": "Cost by role this week", "columns": ["Role", "Runs", "Cost"],
         "rows": [["pm", "12", "$1.40"], ["dev", "30", "$4.10"]]},
        {"kind": "tickets", "ids": ["C001"]},
    ]
    for v in views:
        call("desk.show", {"session": session, "view": v})
elif action == "speak":
    call("voice.relay", {"host": sys.argv[3], "terminal": "t1", "role": sys.argv[4], "text": " ".join(sys.argv[5:])})
