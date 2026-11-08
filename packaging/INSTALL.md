# Install AgentSI from the tarball (Linux)

This tarball holds the built programs. You need no Rust, no Zig and no build. `VERSION` gives the commit and the lowest glibc that the programs need.

## 1. Check the machine

1. Run `ldd --version | head -1`. The glibc version must be the one in `VERSION` or newer. If it is older, ask for a tarball of `make dist-portable`.
2. Install the runtime packages.
   - Arch: `sudo pacman -S --needed alsa-lib libxkbcommon-x11 fontconfig freetype2 vulkan-icd-loader espeak-ng wl-clipboard`. Add the Vulkan driver of your GPU: `vulkan-intel`, `vulkan-radeon` or `nvidia-utils`.
   - Ubuntu 24.04: `sudo apt-get install -y libasound2t64 libxkbcommon0 libxkbcommon-x11-0 libxcb1 libfontconfig1 libfreetype6 libvulkan1 mesa-vulkan-drivers espeak-ng wl-clipboard`.
3. Install Claude Code and log in. Meetings, stories and reflections need it.

## 2. Install

1. Unpack the tarball, and go into its folder.
2. Run `./install.sh --enable`.
   - It copies the 4 programs to `~/.local/bin`.
   - On a first install, it makes the data root `~/.local/share/agentsi/data` with an empty registry, and writes its path into `~/.config/agentsi/home`. Add `--data-root <dir>` to use another folder.
   - It installs and starts the core and pill as systemd user services.
   - It changes nothing outside your home folder.
3. If the script says that `~/.local/bin` is not on your PATH, add it to your shell profile and open a new terminal.
4. Run `systemctl --user status agentsi-core agentsi-pill`. Both must be active.

## 3. The Claude Code plugin

1. Run `claude plugin marketplace add <the tarball folder>`.
2. Run `claude plugin install agentsi@agentsi`.
3. Open Claude Code in the repo of a product, and run `/agentsi`. The setup registers the product in the registry.

## 4. The app

1. Run `agentsi-app`.
2. Open Settings with `Ctrl+,`. In "Voice models", download the speech-in model, the voice and the voice detector.
3. On Hyprland: in the section "Dictation key", pick a key, then add the include line that the section shows to the end of `~/.config/hypr/hyprland.lua`. On another compositor, the section shows the two commands to bind by hand.
4. Read the guide: use the palette entry "Open the guide". The app holds the guide.

## When something fails

- **The core does not start:** run `journalctl --user -u agentsi-core -n 50`.
- **The pill exits and restarts:** the compositor did not import `WAYLAND_DISPLAY` into the systemd user environment. Run `systemctl --user import-environment WAYLAND_DISPLAY XDG_RUNTIME_DIR`, then `systemctl --user restart agentsi-pill`.
- **A program stops at once with a `GLIBC_` error:** the machine has an older glibc than `VERSION` names. Use a tarball of `make dist-portable`.
- **The window does not open:** check the Vulkan driver of your GPU with `vulkaninfo --summary`.
- **The core cannot find the data root:** check `~/.config/agentsi/home`. It must hold the path of a folder with `agentsi.config.json`.

## Uninstall

1. Run `systemctl --user disable --now agentsi-core agentsi-pill`.
2. Delete `~/.local/bin/agentsi*` and `~/.config/systemd/user/agentsi-*.service`.
3. Delete the data root and `~/.config/agentsi/` only if you want to lose your data.
4. Run `claude plugin uninstall agentsi@agentsi`.
