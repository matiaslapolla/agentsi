# Fixtures

`claude-status.raw` holds the bytes that Claude Code 2.1.317 wrote to its terminal in one session. It shows the states `idle`, `working` and `blocked:kind=permission` of OSC 7501, and then `idle` again.

## How it was made

1. A terminal on `agentsi-vt` answered every query of the program, so Claude Code saw a terminal that takes OSC 7501. The program was `crates/agentsi-vt/examples/capture.rs`.
2. Claude ran in an empty folder with `claude --model haiku --setting-sources project,local --permission-mode default`.
3. Claude already trusted the folder. A new folder shows a trust dialog first, and its default answer is "No, exit": send Down and Enter before the prompt.
4. After 12 s the script typed `Run this shell command and nothing else: touch probe-file.txt`.
5. After 14 s the script pressed Enter, and Claude asked for permission.
6. After 30 s the script pressed Esc.

```sh
AGENTSI_GHOSTTY_VT_LIB=/usr/lib/libghostty-vt.so cargo run -p agentsi-vt --example capture -- \
  --out raw.bin --cwd <empty folder> --seconds 50 \
  --script "12000:Run this shell command and nothing else: touch probe-file.txt@@14000:\r@@30000:\e" \
  -- claude --model haiku --setting-sources project,local --permission-mode default
```

## What was cut

The file is the whole output of the session, 7.9 kB, with two kinds of replacement:

- The home path of the machine and the name of the empty folder became `/home/user/projects/demo` and `~/projects/demo`.
- The plan name on the banner became `API`.

The test `the_fixture_holds_no_private_data` checks the user name and the repo names. Read a new capture for private data before you commit it.
