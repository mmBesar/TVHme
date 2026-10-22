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

**Other Linux:** download the AppImage (`TVHme-x86_64.AppImage` or `TVHme-aarch64.AppImage`) or the `tvhme-linux-<arch>.tar.gz` from the Releases page. Needs mpv (or VLC) for playback. RISC-V (`riscv64`) is offered as a tar.gz.

- **AppImage:** `chmod +x` and run. It is portable: it does not touch your system, so a loose AppImage has no menu entry or bar icon (that is how AppImages work everywhere). If you want one, either use an AppImage manager (AppImageLauncher, Gear Lever, ...) or press **Settings > App menu > Add to app menu**, or run `./TVHme-x86_64.AppImage --install-launcher` (undo with `--uninstall-launcher`). This only adds files under `~/.local/share` for your user.
- **tar.gz:** unpack and run `./install.sh` (installs under `~/.local`, undo with `./install.sh --uninstall`).

**Android:** download the APK from the Releases page: `arm64-v8a` for modern phones, `armeabi-v7a` for 32-bit ones, and the `android6` builds for Android 6 (experimental, only published when they build).

## Develop

    cargo test -p tvhme-core -p tvhme-mock -p tvhme-app
    cargo run -p tvhme-mock          # prints ports; users admin/admin and user/user
    cargo run -p tvhme-app --bin tvhme
    nix develop                      # dev shell with the runtime libraries on LD_LIBRARY_PATH
