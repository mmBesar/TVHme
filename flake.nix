{
  description = "TVHme - Tvheadend client (Rust + Slint) for Linux";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        tvhme = pkgs.callPackage ./nix/package.nix { };
        default = tvhme;
      });

      apps = forAll (pkgs: {
        default = { type = "app"; program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.tvhme}/bin/tvhme"; };
      });

      overlays.default = final: _prev: { tvhme = final.callPackage ./nix/package.nix { }; };

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.tvhme ];
          packages = [ pkgs.cargo pkgs.rustc pkgs.clippy pkgs.rust-analyzer pkgs.mpv ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (pkgs.callPackage ./nix/libs.nix { }) + ":/run/opengl-driver/lib";
        };
      });
    };
}
