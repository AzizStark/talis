{
  lib,
  rustPlatform,
  fetchurl,
  runCommand,
  pkg-config,
  wrapGAppsHook4,
  bash,
  coreutils,
  curl,
  cacert,
  glib,
  gtk4,
  gtk4-layer-shell,
  libGL,
  libepoxy,
  librsvg,
  gdk-pixbuf,
  pango,
  cairo,
  graphene,
  libx11,
  libxext,
  wayland,
  adwaita-icon-theme,
  patchelf,
}:
let
  packageVersion = (builtins.fromTOML (builtins.readFile ../../Cargo.toml)).package.version;
  # Scene releases are pinned independently of application source versions.
  sceneVersion = "0.1.0";
  # Reuse the published scene data; build the fixed application from this checkout.
  releaseData =
    runCommand "talis-${sceneVersion}-data"
      {
        src = fetchurl {
          url = "https://github.com/AzizStark/talis/releases/download/v${sceneVersion}/talis-${sceneVersion}-portable-linux-x86_64.tar.gz";
          hash = "sha256-Q/eFEPNieV0wYpsIoPRZHxvKnTDHfP/gEiVYC7Cy/6M=";
        };
      }
      ''
        tar --no-same-owner -xf "$src" talis-linux-x86_64/usr/share
        mkdir -p "$out"
        mv talis-linux-x86_64/usr/share "$out/share"
      '';
in
rustPlatform.buildRustPackage {
  pname = "talis";
  version = packageVersion;
  src = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [
      ../../Cargo.toml
      ../../Cargo.lock
      ../../src
      ../../shaders
      ../../config
      ../../assets/ui
      ../../assets/artwork
      ../../LICENSE
      ../../scripts/launch.sh
      ../../scripts/session.sh
      ../../integrations/gnome/metadata.json
      ../../packaging/io.github.azizstark.Talis.desktop
      ../../packaging/io.github.azizstark.Talis.metainfo.xml
    ];
  };
  cargoLock.lockFile = ../../Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    wrapGAppsHook4
    patchelf
  ];
  buildInputs = [
    gtk4
    glib
    gdk-pixbuf
    librsvg
    pango
    cairo
    graphene
    gtk4-layer-shell
    libGL
    libepoxy
    wayland
    libx11
    libxext
    adwaita-icon-theme
  ];
  dontWrapGApps = true;
  # The application selects the backend, so keep X11/GNOME free of layer-shell.
  buildPhase = ''
    runHook preBuild
    cargo build --offline --locked --release --bin talis --bin talis-gnome-scene
    mkdir -p artifacts
    cp target/release/talis artifacts/talis-bin
    cp target/release/talis-gnome-scene artifacts/talis-gnome-scene
    cargo build --offline --locked --release --features wayland-desktop --bin talis
    cp target/release/talis artifacts/talis-wayland-bin
    runHook postBuild
  '';
  postPatch = ''
    ln -s ${releaseData}/share/talis/assets/scenes assets/scenes
  '';
  checkPhase = ''
    runHook preCheck
    cargo test --offline --locked --release --lib autostart::
    cargo test --offline --locked --release --lib app::startup::
    runHook postCheck
  '';
  installPhase = ''
    runHook preInstall
    mkdir -p "$out/lib/talis" "$out/bin" "$out/share/talis"
    cp -a ${releaseData}/share/{applications,icons,metainfo,doc} "$out/share/"
    # Only scene data is pinned to an older release; use current app metadata.
    chmod u+w "$out/share/applications" "$out/share/metainfo"
    install -m644 packaging/io.github.azizstark.Talis.desktop "$out/share/applications/"
    install -m644 packaging/io.github.azizstark.Talis.metainfo.xml "$out/share/metainfo/"
    ln -s ${releaseData}/share/talis/assets "$out/share/talis/assets"
    cp artifacts/* scripts/{launch.sh,session.sh} "$out/lib/talis/"
    chmod +x "$out/lib/talis/launch.sh"
    ln -s ../lib/talis/launch.sh "$out/bin/talis"
    ln -s ../lib/talis/talis-gnome-scene "$out/bin/talis-gnome-scene"
    substituteInPlace "$out/share/applications/io.github.azizstark.Talis.desktop" \
      --replace-fail "Exec=talis" "Exec=$out/bin/talis"
    extension="$out/share/gnome-shell/extensions/talis@azizstark"
    mkdir -p "$extension"
    cp ${releaseData}/share/talis/gnome/*.{js,json} ${releaseData}/share/talis/gnome/LICENSE "$extension/"
    ln -s "$out/share/talis/assets" "$extension/assets"
    ln -s "$out/bin/talis-gnome-scene" "$extension/talis-gnome-scene"
    runHook postInstall
  '';
  postFixup = ''
    for program in talis-bin talis-wayland-bin talis-gnome-scene; do
      patchelf --add-rpath "${
        lib.makeLibraryPath [
          libGL
          libepoxy
          libx11
          libxext
          wayland
        ]
      }" "$out/lib/talis/$program"
      # Each backend's startup entry must restore the same GTK environment.
      wrapProgram "$out/lib/talis/$program" "''${gappsWrapperArgs[@]}" \
        --prefix PATH : "${
          lib.makeBinPath [
            bash
            coreutils
            curl
            glib
          ]
        }" \
        --set-default SSL_CERT_FILE "${cacert}/etc/ssl/certs/ca-bundle.crt" \
        --set TALIS_STARTUP_EXECUTABLE "$out/lib/talis/$program"
    done
    wrapProgram "$out/lib/talis/launch.sh" \
      --prefix PATH : "${
        lib.makeBinPath [
          bash
          coreutils
          glib
        ]
      }" \
      --set-default TALIS_LAYER_SHELL_LIB "${gtk4-layer-shell}/lib/libgtk4-layer-shell.so.0"
  '';
  passthru.extensionUuid = "talis@azizstark";
  meta = {
    description = "An interactive, animated and music-aware desktop turntable";
    homepage = "https://github.com/AzizStark/talis";
    # Full scene and third-party terms are retained in share/doc/talis/LICENSING.md.
    license = with lib.licenses; [
      gpl3Only
      cc-by-40
      mit
    ];
    platforms = [ "x86_64-linux" ];
    mainProgram = "talis";
  };
}
