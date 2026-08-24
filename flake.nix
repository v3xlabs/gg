{
  description = "A local Git client with a repository switcher and a commit graph";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = ["x86_64-linux" "aarch64-linux" "aarch64-darwin" "x86_64-darwin"];

      perSystem = {pkgs, ...}: {
        packages.default = import ./nix/package.nix {inherit pkgs inputs;};
        checks = import ./nix/checks.nix {inherit pkgs inputs;};
        devShells = {
          default = import ./nix/shell.nix {inherit pkgs inputs;};

          # Carries a virtual X display so the interface can be driven and captured
          # with no real screen involved. Separate from the default shell so nobody
          # pays for it during ordinary development.
          testing = import ./nix/shell.nix {
            inherit pkgs inputs;
            extraPackages = [
              pkgs.xvfb
              pkgs.xdotool
              pkgs.imagemagick
              pkgs.x11vnc
              pkgs.tigervnc
            ];
          };
        };
      };

      flake.overlays.default = final: _prev: {
        gg = import ./nix/package.nix {
          pkgs = final;
          inherit inputs;
        };
      };
    };
}
