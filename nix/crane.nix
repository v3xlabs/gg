{
  pkgs,
  inputs,
}: let
  inherit (pkgs) lib stdenv;

  craneLib = inputs.crane.mkLib pkgs;

  # wgpu and winit load these with dlopen rather than linking them, so they have to be on
  # the library path at run time as well as at build time.
  runtimeLibraries = lib.optionals stdenv.hostPlatform.isLinux (with pkgs; [
    libxkbcommon
    vulkan-loader
    wayland
    libx11
    libxcursor
    libxi
    libxrandr
  ]);

  # cleanCargoSource keeps Rust and cargo files only, but the file type icons under
  # assets/icons are include_bytes! inputs of the compile itself, so they have to survive
  # the filter. This lives here rather than on the package so the dependency build and
  # the checks compile the same source.
  source = lib.cleanSourceWith {
    src = lib.cleanSource ./..;
    name = "source";
    filter = path: type:
      # .tmp holds throwaway fixtures, and some of them are cargo projects of their own,
      # which filterCargoSources would otherwise pull into the build.
      !(lib.hasInfix "/.tmp/" path)
      && (craneLib.filterCargoSources path type || lib.hasInfix "/assets/icons/" path);
  };

  commonArgs = {
    src = source;
    strictDeps = true;

    nativeBuildInputs = [pkgs.pkg-config];
    buildInputs = runtimeLibraries;

    # The tests in src/git/command.rs spawn git, which is the point of that module.
    nativeCheckInputs = [pkgs.git];
  };
in {
  inherit craneLib commonArgs runtimeLibraries;
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
}
