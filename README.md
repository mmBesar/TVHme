<p align="center"><img src="assets/logo.png" alt="TVHme" width="520"></p>

# TVHme

A modern Tvheadend client for Linux and Android. Rust core, Slint UI.

- Admins get management screens; normal users get what their account allows (the server decides).
- Playback uses Tvheadend's HTTP stream; signal (SNR/SS %) comes from HTSP `signalStatus`.
- Catppuccin themes (default Macchiato + Mauve), AMOLED black, system look.

Status: early development.

## Install

**NixOS / Nix** (the app loads Wayland/X11/GL libraries at runtime and the package wraps them in; it builds from source, so expect a large first download):

    nix run github:mmBesar/TVHme

Or add `inputs.tvhme.url = "github:mmBesar/TVHme";` to your flake and use `inputs.tvhme.packages.${system}.default`.

**Other Linux:** download the AppImage (`TVHme-x86_64.AppImage` or `TVHme-aarch64.AppImage`) or the `tvhme-linux-<arch>.tar.gz` from the Releases page. Needs mpv (or VLC) for playback. RISC-V (`riscv64`) is a tar.gz built on request.

- **AppImage:** `chmod +x` and run. It is portable: it does not touch your system, so a loose AppImage has no menu entry or bar icon (that is how AppImages work everywhere). If you want one, either use an AppImage manager (AppImageLauncher, Gear Lever, ...) or press **Settings > App menu > Add to app menu**, or run `./TVHme-x86_64.AppImage --install-launcher` (undo with `--uninstall-launcher`). This only adds files under `~/.local/share` for your user.
- **Fedora (44 and later):** download `tvhme-<version>-1.fc44.x86_64.rpm` or `...aarch64.rpm` and run `sudo dnf install ./tvhme-*.rpm`. It gives you the menu entry, icons and `tvhme` command like any other package.
- **tar.gz:** unpack and run `./install.sh` (installs under `~/.local`, undo with `./install.sh --uninstall`).

**Android:** download the APK from the Releases page: `arm64-v8a` for modern phones, `armeabi-v7a` for 32-bit ones. The minimum is Android 8 (API 26): the Skia renderer that Slint uses on Android is built for that level.

## Develop

    cargo test -p tvhme-core -p tvhme-mock -p tvhme-app
    cargo run -p tvhme-mock          # prints ports; users admin/admin and user/user
    cargo run -p tvhme-app --bin tvhme
    nix develop                      # dev shell with the runtime libraries on LD_LIBRARY_PATH
