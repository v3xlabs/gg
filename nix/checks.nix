{
  pkgs,
  inputs,
}: let
  inherit (import ./crane.nix {inherit pkgs inputs;}) craneLib commonArgs cargoArtifacts;
in {
  clippy = craneLib.cargoClippy (commonArgs
    // {
      inherit cargoArtifacts;
      cargoClippyExtraArgs = "--all-targets -- --deny warnings";
    });

  tests = craneLib.cargoTest (commonArgs // {inherit cargoArtifacts;});

  formatting = craneLib.cargoFmt {inherit (commonArgs) src;};
}
