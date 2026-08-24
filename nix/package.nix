{
  pkgs,
  inputs,
}: let
  inherit (pkgs) lib;
  inherit (import ./crane.nix {inherit pkgs inputs;}) craneLib commonArgs cargoArtifacts runtimeLibraries;
in
  craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts;

      nativeBuildInputs = commonArgs.nativeBuildInputs ++ [pkgs.makeWrapper];

      # The assets arrive as store paths rather than from $src, because cleanCargoSource
      # deliberately filters out everything that is not Rust or cargo.
      postInstall =
        # The name it was published under before, so `gitgui` still starts it.
        ''
          ln -s gg $out/bin/gitgui
        ''
        + lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
          wrapProgram $out/bin/gg \
            --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibraries}

          install -Dm444 ${../assets/gg.desktop} \
            $out/share/applications/gg.desktop
          install -Dm444 ${../assets/gg.svg} \
            $out/share/icons/hicolor/scalable/apps/gg.svg
        '';

      meta = {
        description = "A local Git client with a repository switcher and a commit graph";
        homepage = "https://github.com/v3xlabs/gg";
        license = lib.licenses.mit;
        mainProgram = "gg";
      };
    })
