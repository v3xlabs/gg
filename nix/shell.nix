{
  pkgs,
  inputs,
  extraPackages ? [],
}: let
  inherit (pkgs) lib;
  inherit (import ./crane.nix {inherit pkgs inputs;}) craneLib commonArgs runtimeLibraries;
in
  craneLib.devShell {
    inherit (commonArgs) nativeBuildInputs buildInputs;

    packages =
      [
        pkgs.git
        pkgs.rust-analyzer
        # scripts/test.sh runs every test binary inside this, so the git those tests
        # spawn cannot reach the machine's own configuration or its keys.
        pkgs.bubblewrap
      ]
      ++ extraPackages;

    LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibraries;
  }
