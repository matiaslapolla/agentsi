# An old call

`call.webm` and `timings.json` come from the Python package `packages/voice`, run on synthetic text:

```
agentsi-voice render script.json <out>
```

`script.json` held three segments: pm "Welcome to the test meeting. Today we check the call.", dev "The build is green, and the parser bug is fixed." and qa "I ran the card tests twice. Both runs passed."

The package encoded them with `ffmpeg` (libopus, 32 kbit/s, VoIP) into WebM. The call is 9.356 s long.

`tests/call_old.rs` plays this file through the same code as a new call and compares it with a reference decode by `ffmpeg -c:a libopus`.
