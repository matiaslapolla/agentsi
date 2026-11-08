# agentsi-vt

libghostty-vt behind a small safe API. A `Terminal` parses the bytes of a program and answers its queries. It also reports the agent state of OSC 7501, copies the visible grid and encodes keys.

```rust
let mut term = agentsi_vt::Terminal::new(agentsi_vt::Options::default())?;
term.write(bytes_from_the_pty);
for event in term.take_events() {
    match event {
        Event::Reply(bytes) => pty.write(&bytes)?, // in this order
        Event::Status(report) => { /* idle, working, blocked ... */ }
        _ => {}
    }
}
let grid = term.grid()?;
```

`replay()` gives the bytes that bring a new terminal to the same state: the screen and its scrollback, the modes, the styles, the cursor, and the sequence that the input left unfinished. It has no palette, so a pane keeps its theme. The core sends them as the first paint of a pane. It holds the active screen only. A replay made on the alternate screen does not hold the main screen behind it. `mode(n)` reads a DEC private mode, such as 1004 (focus reports).

`grid()` copies every visible row. `update_grid(&mut grid)` brings a kept grid up to date and reads again only the rows that libghostty-vt marks dirty. It returns a `Damage`: the rows it read, `all` when it read every row, and the id of each row. A row keeps its id when a scroll moves it, so a renderer can find its old copy. A scroll of the screen marks every row dirty. `grid()` leaves the dirty marks for the next `update_grid`.

For a pane, the terminal also moves the viewport (`scroll`, `scrollbar`), reads modes (`mode`, `mouse_tracking`, `alternate_screen`), sets the default colors (`set_palette`) and encodes mouse events, pastes and focus changes (`encode_mouse`, `encode_paste`, `encode_focus`).

## Build

`build.rs` links libghostty-vt in one of two ways.

### 1. Build Ghostty with the pinned Zig (default)

```sh
cargo build -p agentsi-vt
```

- The Ghostty commit is in `ghostty.lock`, with the SHA-256 of its source tarball.
- `build.rs` downloads the tarball with `curl` into `~/.cache/agentsi/ghostty/` once, checks the hash, and extracts it into `OUT_DIR`.
- `scripts/zig.sh` gives the Zig 0.17.0 binary. Order: `$AGENTSI_ZIG`, a `zig` on the `PATH` that reports 0.17.0, a checksummed download into `~/.cache/agentsi/zig/0.17.0/`.
- `zig build -Demit-lib-vt -Doptimize=ReleaseFast` makes a static `libghostty-vt.a`, which the crate links.
- Without network, set `AGENTSI_GHOSTTY_SOURCE_DIR` to an extracted source tree of the same commit.
- Cold build, Linux x86_64, 12 cores, empty cargo, Zig and Ghostty caches: 2 min 52 s. This includes the download and the Zig compile of Ghostty. Later builds take seconds.

### 2. Link a library file (no Zig)

```sh
AGENTSI_GHOSTTY_VT_LIB=/usr/lib/libghostty-vt.so cargo build -p agentsi-vt
```

- The file must be named `lib<name>.so`, `.dylib` or `.a`. The first two link dynamically, `.a` links statically. A versioned name such as `libghostty-vt.so.1` fails: add an unversioned link next to it.
- A dynamic library gets an rpath to its folder. The rpath reaches the tests and examples of this crate only. A program in another crate, such as the core, needs the folder in `LD_LIBRARY_PATH`, or the library in a standard folder.
- The library must come from the same Ghostty commit as `ghostty.lock`, or from a commit with the same C API. The Arch package `ghostty` 1.4.0 works with the pinned commit.
- `agentsi_vt::LIBRARY_SOURCE` tells which way built the binary.

## Upgrade Ghostty

1. Change the five lines of `ghostty.lock`.
2. Regenerate `src/sys.rs` from the new headers with rust-bindgen (see the header of the file).
3. Run `cargo test -p agentsi-vt` on both build paths.

## Threading

`Terminal` is `Send` and not `Sync`. One thread at a time calls it. Callbacks of the library run inside `write` on the calling thread.

## Code map

| File | Contents |
| --- | --- |
| `src/terminal.rs` | The only code that calls the library. Each `unsafe` block has a safety note. |
| `src/sys.rs` | Raw declarations, generated. |
| `src/status.rs` | `AgentState`, `StatusReport` and the records of the protocol. |
| `src/key.rs`, `src/input.rs`, `src/grid.rs` | Plain types for key and mouse events, the copied screen, the palette and the viewport. |
| `examples/capture.rs` | Runs a program behind a `Terminal` and records its output. |
| `tests/` | Behavior tests, and the replay of a real Claude Code session (`tests/fixtures/`). |
