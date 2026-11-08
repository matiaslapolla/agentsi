# AgentSI desktop: the guide and the way to the call

**Status:** approved 2026-11-07 · **Source:** owner decisions of 2026-11-07 (log "Owner decisions after the first owner checks")

## Problem

The guide is in Spanish and describes the web app, Postgres and `make dev`. `/como-funciona` lives in the web app, which step 7 deletes. The app plays a meeting's call on the Call screen, but no button on the minutes leads there, so the owner cannot find it.

## Goal

- The repo holds one English guide for the desktop app, which the app opens from the palette and from a Help entry.
- The owner opens the call of a minutes from the minutes list, the minutes screen and the meeting screen with one click.

## Non-goals

- A help screen drawn inside the app. The guide stays a local HTML file, opened in the browser as today (`guide.rs`).
- A Spanish copy.
- New call features. The player of U26 stays as it is.

## Terms

No new terms.

## Design

### Guide content

- `docs/guide/` gets one English guide for the desktop app. It replaces `index.html` and takes the content of `/como-funciona` that is still true.
- Sections:
  1. What AgentSI is: the roles advise, the owner decides, the data stays local and in git.
  2. Install and start: `packaging/install.sh`, the core service, `agentsi-app`, `agentsi doctor`.
  3. The window: sidebar tree, tabs, palette (`Ctrl+K`), settings.
  4. Products: the registry, `agentsi whoami`, add a product.
  5. Meetings: start one, read the minutes, listen to the call.
  6. Decisions and priorities: the desk, mark, decide.
  7. Stories: propose, run, review, accept or reject.
  8. Roles learn: reflect, memory, brief, audit.
  9. Voice: dictation, toggle dictation, conversation mode, voice mode, the vocabulary.
  10. Keys: the Keys section, the dictation key, the include line.
  11. Data: the data root, backups, clear data.
  12. The CLI and the Claude Code plugin: `agentsi help`, `/agentsi`, `/agentsi:voice`.
- The four diagrams of `/como-funciona` that still hold (what AgentSI touches in your repo, a meeting, a story, how the roles learn) come back as inline SVG with the words of `CONTEXT.md`. The diagram of the pieces is drawn again: app, core, pill, SQLite, plugin.
- Each section that has a screen gets a screenshot of the real app on synthetic data, taken with `grim -T <window id>` in a session that is not the owner's session. The four web PNGs go.
- The text obeys the STE rules of the repo docs. The page follows the light and dark mode of the system and uses the color values of `agentsi-ui` tokens, written by hand in the CSS.
- `scripts/guide.ts` and `make guide` go, because they drive the TypeScript CLI. A new `scripts/guide-shots.sh` takes the screenshots again. The text is written by hand.
- If the switch-over has run, the content of `/como-funciona` comes from the tag `last-typescript`.

### Guide in the app

- The two palette entries "Open the guide" stay, and open the new page and its section "Install and start".
- The settings screen gets a line "Guide" with an Open button.
- README.md links the guide and drops the web links.

### The way to the call

- Each row of the Minutes screen that has a call shows a play mark. A click on it opens the Call screen.
- The minutes screen gets a "Listen" button when a call exists, and a "Make the call" button when none exists. "Make the call" runs `call.render` and shows its progress from `meeting.render`, as the Call screen does.
- The meeting screen shows "Listen" after its minutes have a call.
- `Ctrl+Shift+P` stays.

## Acceptance criteria

- [ ] `docs/guide/index.html` is in English, names no web screen, Postgres or `make dev`, and covers the 12 sections. (The amendment of 2026-11-08 moves this to `docs/guide/README.md`.)
- [ ] Each link and image of the guide resolves to a file in the repo.
- [ ] The page passes the contrast rule (4.5:1) in light and in dark mode.
- [ ] The palette entries and the settings line open the guide.
- [ ] A test of the real Minutes screen with a fake core opens the Call screen from a play mark.
- [ ] A test of the minutes screen shows "Listen" with a call and "Make the call" with none, and "Make the call" runs `call.render`.
- [ ] `git diff --stat` shows no change to `schema/core.openrpc.json`.
- [ ] The owner reads the guide and opens a call from the minutes.

## Amendment 2026-11-08: the guide in the app

**Status:** approved 2026-11-08 · **Source:** owner decision of 2026-11-08. It replaces the non-goal "A help screen drawn inside the app" and the browser path of the sections "Guide content" and "Guide in the app".

### Problem

The guide opens in the browser from a file. An installed app with no checkout cannot find the file, and the owner leaves the app to read it.

### Design

- `docs/guide/README.md` is the one source of the guide. It is GitHub-flavored Markdown with the same sections as the old page, and it uses the images of `docs/guide/img/` with relative links. GitHub renders the same file. `docs/guide/index.html` goes.
- The five diagrams of the old page become numbered steps or lists, because the Markdown page draws no SVG. Each keeps its caption.
- A key is an inline code span, for example `Ctrl+K`, because the Markdown view does not draw `<kbd>`.
- The heading of a section gives its anchor, as on GitHub. The old numbers in the headings go, so the anchors stay `what-agentsi-is`, `install-and-start` and so on. The section 12 anchor is now `the-cli-and-the-claude-code-plugin`.
- The app has a Guide screen: a tab like the other screens, with no product. It draws the Markdown with `TextView` of gpui-component. The page scrolls. Page Up, Page Down, Home and End scroll it, as on the Settings screen.
- The program embeds the Markdown and the 18 PNG files at compile time. The screen maps each `img/<file>.png` link to the embedded bytes. An installed app with no checkout shows the guide with its images.
- A click on a link to http or https opens the browser. A click on a link `#<anchor>` scrolls the page to that heading. A link to a file of the repo (`../../CONTRIBUTING.md`) does nothing in the app, because an installed app has no checkout.
- The palette entry "Open the guide: how AgentSI works" and the line "Guide" of the Settings screen open the Guide screen at the top. The entry "Open the guide: the first steps" opens it scrolled to "Install and start".
- The browser path goes: `AGENTSI_GUIDE`, the search for `docs/guide/index.html` and the URL of the guide. `file_url` stays, because "Open folder" of a product uses it.
- The tarball of `make dist` no longer holds a `guide/` folder, because the program holds the guide. `make guide` opens `docs/guide/README.md` with the default program.
- The color test of `index.html` (`crates/agentsi-ui/tests/guide_colors.rs`) goes, because the page has no CSS. The Markdown view takes the colors of the theme, which `agentsi-ui` sets.

### Acceptance criteria

- [x] `docs/guide/README.md` holds each heading of the sections and subsections of the old page, and each of the 18 images with its alt text.
- [x] The Markdown is in English, names no web screen, Postgres or `make dev`, and has no semicolon in its prose.
- [x] Each relative link of the guide resolves to a file or to a heading of the page.
- [x] Each image link of the guide resolves to an embedded image.
- [x] The palette entry "Open the guide: how AgentSI works" opens the Guide screen.
- [x] The line "Guide" of the Settings screen opens the Guide screen.
- [x] The palette entry "Open the guide: the first steps" opens the Guide screen scrolled to "Install and start".
- [x] Page Down, Page Up, End and Home scroll the Guide screen.
- [x] A link to http or https opens the browser. A link `#<anchor>` scrolls the page.
- [x] No live file refers to `docs/guide/index.html`. The logs, the reviews and the old text of this spec keep it as history. `README.md` at the root is rewritten by another unit.
- [ ] OWNER: The owner reads the guide in the app, with its images, from an install with no checkout.
