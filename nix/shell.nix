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
      ]
      ++ extraPackages;

    LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibraries;
  }
