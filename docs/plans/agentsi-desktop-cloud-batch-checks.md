# Owner checks: the desktop batch of 2026-11-21

The build of `main` 5314936 is installed (`make install` of 2026-11-22). Open the app from the launcher, or run `agentsi-app`. Each item says what to check and how. Tick it when it works.

## U1 — PATH (#6)

- [ ] **A meeting finds `claude`.** Open the front desk of a product (`Ctrl+Shift+D`) and start a meeting. It must run, with no "cannot find `claude`" error.
- [ ] **The doctor shows the merged PATH.** Run `agentsi doctor` in a terminal. The `core` row says `PATH merged`, and the `claude` row shows its full path.

## U2 — Settings sections, fonts and terminal (#8)

- [ ] **The section list.** Open Settings (`Ctrl+,`). Click "Git" in the left list: only the Git section shows, and Up and Down move in the list.
- [ ] **The last section stays.** Open the Terminal section, close the app and open it again. Settings opens at Terminal.
- [ ] **The palette opens a section.** Press `Ctrl+K` and type `#term`. Enter opens Settings at Terminal.
- [ ] **The font families.** In Fonts, change the app font and the mono font to other installed families. The app and the open terminals change at once.
- [ ] **The text offset.** Set the text size to +4, then to −2. Every screen still fits, and no text is cut.
- [ ] **The terminal font size.** With a terminal open, change the font size from 13 to 16. Run `tput cols` in that terminal: the number of columns is smaller.
- [ ] **The cursor and the palette.** Set the cursor style to "bar", then the palette to light. The open terminals change at once.
- [ ] **New panes.** Change the scrollback or the shell, then open a new terminal. The new terminal uses the new value.

## U3 — General roles (#7)

- [ ] **Create a general role.** Open Roles (`Ctrl+Shift+R`) and click "New general role". Make `designer`: the file `~/.local/share/agentsi/data/plugin/agents/designer.md` appears.
- [ ] **Two products use it.** In the roster picker of two products, tick `designer`. Both rosters show it, and a meeting of one product cites it.
- [ ] **A used role cannot go.** Try to delete `designer` while a roster uses it. The app refuses and names the products.
- [ ] **Make general.** On a custom role of one product, click "Make general". The role moves to the general section, and the roster still shows it.

## U4 — Spaces, pinned tabs and favorites (#12)

- [ ] **One space for each product.** Open a terminal in product A, then select product B. The top bar shows only B's tabs. Go back to A: the terminal still runs.
- [ ] **The tabs in the sidebar.** Under each product in the sidebar, its open tabs show. A click on a tab row selects the product and focuses that tab.
- [ ] **Pin a tab.** Right-click a tab and pin it. It moves to the left, gets narrow, and survives "Close others". Closing it asks for a confirm.
- [ ] **A favorite.** Right-click a terminal and add it to favorites. Click the favorite in the sidebar: a terminal opens with the same command and folder.
- [ ] **After a restart.** Close and open the app. The spaces, the pinned tabs and the favorites are the same.
- [ ] **Next space.** Press `Ctrl+Tab`. The next product shows.

## U5 — Terminal links (#10)

- [ ] **A URL.** In a terminal, run `echo https://example.com`. Hold Ctrl over it: it gets an underline. Ctrl+click opens the system browser.
- [ ] **An OSC 8 link.** Run `printf '\e]8;;https://example.com\e\\link\e]8;;\e\\\n'`. Ctrl+click on "link" opens the browser.
- [ ] **A file and a line.** Run `echo src/main.rs:12` in a repo that has that file. Ctrl+click opens `$EDITOR +12 src/main.rs` in a new terminal, in the same product.
- [ ] **A plain click.** Without Ctrl, a click on a link starts a selection, as before.
- [ ] **The link menu.** Right-click on a link. The menu has "Open link" and "Copy link".

## U6 — Git (#11)

- [ ] **The diff of a commit.** Open Git (`Ctrl+Shift+B`) and go to History. A click on a commit shows its files and its diff.
- [ ] **Branches.** Open the Branches tab. Switch to another branch with a clean tree: it works. With a changed file, the app says `dirty` and lists the file.
- [ ] **Push.** Make a commit and click Push. A confirm lists the outgoing commits. After the confirm, the commit is on the remote.
- [ ] **Pull with a diverged branch.** On a branch that diverged from its remote, click Pull. The app says to use a terminal, and nothing changes.
- [ ] **No CPU when idle.** Leave the app open on a big repo for a few minutes. `top` shows `agentsi-core` near 0 % CPU.

## U7 — File explorer (#14)

- [ ] **The Files mode.** Press `Ctrl+Shift+E`, or use the switch at the top of the sidebar. The tree of the selected product shows.
- [ ] **New folder and new file.** Right-click a folder and choose "New folder". Type a name and press Enter: the folder appears.
- [ ] **A change from a terminal.** Run `touch x.txt` in a terminal of the product. The file shows in the tree in less than 1 s.
- [ ] **Move to trash.** Right-click the file and choose "Move to trash". It goes to the system trash, not away for good.
- [ ] **Open a terminal here.** Right-click a folder and choose "Open terminal here". A terminal opens in that folder, in the space of the product.
- [ ] **Ignored files.** A file that `.gitignore` ignores shows in the ignored color. The eye button hides it.

## U8 — Git marks (#13)

- [ ] **The product rows.** In Products mode, each product row shows its branch. With changes, it shows `·N`, and `↑a ↓b` when it is ahead or behind.
- [ ] **The tab dot.** Change a file in the folder of a terminal. Its tab shows a small dot, and the tooltip shows the git summary.

## U9 — Palette (#15)

- [ ] **Tabs.** Press `Ctrl+K` and type `@` and part of a tab name. Enter focuses that tab, in its product.
- [ ] **Files.** Type `/` and part of a file name. The files of the selected product show, and Enter opens one.
- [ ] **Recent items.** Open `Ctrl+K` with an empty query. The last targets you opened come first.
- [ ] **A second step.** Choose "Git: Switch branch…". A list of branches shows, and Escape goes back to the first step.
- [ ] **Reopen a tab.** Close a tab, then press `Ctrl+Alt+T`. The tab comes back in its product.
- [ ] **Restart core, with care.** "Restart core" relaunches the app and ends every terminal of this device. Try it only with no work open.

## Idle frames

- [ ] **The idle bench.** Run `agentsi-app --bench idle` and, for Settings, `agentsi-app --bench idle --screen settings`. Both report 0 frames while idle.
