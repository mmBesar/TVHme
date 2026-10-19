{ lib, callPackage, rustPlatform, pkg-config, makeWrapper, fontconfig, freetype, libxkbcommon, mpv }:
let
  runtimeLibs = callPackage ./libs.nix { };
in
rustPlatform.buildRustPackage {
  pname = "tvhme";
  version = "0.1.0";

  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: type: !(builtins.elem (baseNameOf path) [ "target" ".git" "result" ]);
  };
  cargoLock.lockFile = ../Cargo.lock;

  cargoBuildFlags = [ "-p" "tvhme-app" "--bin" "tvhme" ];
  # Tests run in CI (cargo test); they bind local ports, so skip inside the Nix sandbox.
  doCheck = false;

  nativeBuildInputs = [ pkg-config makeWrapper ];
  buildInputs = [ fontconfig freetype libxkbcommon ];

  postInstall = ''
    install -Dm644 assets/icon.svg $out/share/icons/hicolor/scalable/apps/tvhme.svg
    install -Dm644 packaging/linux/tvhme.desktop $out/share/applications/tvhme.desktop
    wrapProgram $out/bin/tvhme \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}:/run/opengl-driver/lib \
      --prefix PATH : ${lib.makeBinPath [ mpv ]}
  '';

  meta = {
    description = "Tvheadend client for Linux and Android (Rust + Slint)";
    homepage = "https://github.com/mmBesar/TVHme";
    license = lib.licenses.gpl3Plus;
    mainProgram = "tvhme";
    platforms = lib.platforms.linux;
  };
}
