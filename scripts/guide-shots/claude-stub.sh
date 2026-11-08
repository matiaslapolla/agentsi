#!/bin/sh
# A stand-in for `claude` in the screenshots of the guide (scripts/guide-shots.sh puts it first on the
# PATH of the throwaway core). It prints a short stream-json for one role of a meeting, then waits,
# so the meeting stays live. It calls no service.
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
