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

**Other Linux:** download `TVHme-x86_64.AppImage` (or `aarch64`) from the Releases page, `chmod +x`, run. Needs mpv or vlc for playback.

**Android:** debug APK from the Android workflow artifacts.

## Develop

    cargo test -p tvhme-core -p tvhme-mock -p tvhme-app
    cargo run -p tvhme-mock          # prints ports; users admin/admin and user/user
    cargo run -p tvhme-app --bin tvhme
    nix develop                      # dev shell with the runtime libraries on LD_LIBRARY_PATH
