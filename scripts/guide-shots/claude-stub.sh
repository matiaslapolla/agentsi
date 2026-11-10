#!/bin/sh
# A stand-in for `claude` in the screenshots of the guide (scripts/guide-shots.sh puts it first on the
# PATH of the throwaway core). It prints a short stream-json for one role of a meeting, then waits,
# so the meeting stays live. In the desk session it answers each owner line. It calls no service.
case "$1" in auth) echo '{"loggedIn": true, "authMethod": "claude.ai"}'; exit 0 ;; esac
case "$*" in --version) echo "2.1.0 (Claude Code)"; exit 0 ;; esac
case "$*" in *--input-format*)
  # The desk session: one streamed reply for each owner line.
  d() { printf '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"%s"}}}\n' "$1"; }
  echo '{"type":"system","subtype":"init"}'
  while IFS= read -r _; do
    d "Two decisions wait for **you**. "; sleep 0.3
    d "The retry rule is the main priority, "; sleep 0.3
    d "and the pilot costs less than planned. "; sleep 0.3
    echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Two decisions wait for **you**. The retry rule is the main priority, and the pilot costs less than planned."}]}}'
    echo '{"type":"result","is_error":false,"total_cost_usd":0.01,"duration_ms":1300}'
  done
  exit 0 ;;
esac
cat > /dev/null &
tool() { printf '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"%s","input":{"%s":"%s"}}]}}\n' "$1" "$2" "$3"; }
text() { printf '{"type":"assistant","message":{"content":[{"type":"text","text":"%s"}]}}\n' "$1"; }
case "$*" in
  *"**PM**"*)
    tool Read file_path docs/vision.md; sleep 1
    text "The goal is the first paying clinic."; sleep 1
    text "The reminder flow is the request that users repeat most."; sleep 1
    text "I rank the reminder queue first." ;;
  *"**dev**"*)
    tool Grep pattern retry; sleep 1
    tool Read file_path src/queue.ts; sleep 1
    text "The queue drops a job after one failure."; sleep 1
    text "One retry rule closes the gap." ;;
  *)
    tool Glob pattern "tests/**"; sleep 1
    tool Read file_path tests/reminder.test.ts; sleep 1
    text "Two paths have no test."; sleep 1
    text "A late reminder and a canceled booking." ;;
esac
sleep 900
