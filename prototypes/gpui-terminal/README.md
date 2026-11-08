# Prototype: a libghostty-vt terminal pane in GPUI (open test 3)

Throwaway prototype for open test 3 of `docs/ideas/agentsi-desktop.md`: does a Claude Code session render and take input correctly in a libghostty-vt pane in GPUI?

One window, one pane. This code opens the PTY (`libc::openpty`, no `portable-pty`), feeds the output to `ghostty-vt` 0.8.0, and draws the grid with GPUI quads and shaped text.

## Run

Linux with Wayland. `ghostty-vt-sys` builds Ghostty with Zig 0.17. This machine has no Zig, so the prototype links the installed `/usr/lib/libghostty-vt.so` (Arch package `ghostty` 1.4.0) through a patched copy of the sys crate:

```sh
./scripts/vendor-sys.sh      # writes vendor/ghostty-vt-sys (git-ignored)
cargo build --release
./target/release/gpui-terminal -- bash -i
./target/release/gpui-terminal --cwd <trusted folder> -- claude --model haiku
```

With Zig 0.17 installed, delete the `[patch.crates-io]` table in `Cargo.toml` and the crate builds Ghostty itself.

Flags:

| Flag | Meaning |
| --- | --- |
| `--script "ms:text@@ms:text"` | After each delay, write the bytes to the PTY as if typed. Escapes: `\r \n \t \e \xNN \\`. |
| `--exit-after SECS` | Kill the child, write the stats and quit. |
| `--stats FILE` | Append one sample a second (CPU, RSS, frames), events, and a final latency summary. |
| `--dump FILE` | At exit, write the screen as plain text. |
| `--raw FILE` | Append every byte the child wrote. |
| `--size WxH`, `--cwd DIR` | Window size in px and child working folder. |

## What it does

- `pty.rs`: `openpty`, `setsid` + `TIOCSCTTY` in the child, a signal reset (see the findings), `TIOCSWINSZ` on resize, a reader thread.
- `main.rs`: feeds `Terminal::vt_write`, writes `PtyWrite` replies back, reads the grid with `RenderState` and row and cell iterators, and paints background quads, text runs (`shape_line` with a forced cell width), block elements U+2580..U+259F as quads, and the cursor.
- Keys: GPUI `KeyDownEvent` goes to the `ghostty_vt::key::Encoder`, which reads the terminal modes (cursor keys, Kitty flags).
- A side scanner reads `OSC 7501` and `OSC 21337`, which stock libghostty-vt ignores. It answers the `OSC 7501 ; ?` query that Claude Code sends.

## Result, 2026-11-02

See the report of the unit for the evidence and the numbers. In short, on Linux (Hyprland), `gpui-pre` 0.3.11, `ghostty-vt` 0.8.0:

- bash and btop render with colors and redraws: pass. `htop` is not installed here.
- `claude --model haiku` renders (welcome box, prompt, spinner, statusline, answer): pass.
- Input: scripted bytes and real key events both work: pass.
- Resize reflows Claude and btop: pass.
- Frame cost: a few ms in the PTY-to-paint call, about 2 % of one core while the Claude spinner runs, about 165 MB RSS.

Not done: mouse input, scrollback, selection, copy and paste, focus events, IME, macOS.
