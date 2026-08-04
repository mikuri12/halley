# Halley-Mikuri — Rust derivation for the patched Halley compositor.
#
# Differences vs the upstream pkgs/halley.nix:
#   - src is the local ../src tree (already patched with halley-fixes.patch),
#     not a fetchFromGitHub fetch.
#   - No `patches = [...]`: the tree is already patched in this repo.
#   - No `-C target-cpu=native` or LTO/codegen tuning: those are per-machine
#     optimizations and don't belong in a public package. EGL/wayland-client
#     link-args stay (see comments below); without them Halley can't dlopen
#     EGL without polluting the child environment via LD_LIBRARY_PATH.
#   - No HD530-specific MESA_LOADER_DRIVER_OVERRIDE / INTEL_DEBUG env: those
#     were the original developer's tuning.
{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  dbus,
  wayland,
  libxkbcommon,
  libinput,
  seatd,
  udev,
  libgbm,
  libdrm,
  libglvnd,
  pixman,
  pipewire,
  xwayland-satellite,
}: let
  smithayHash = "sha256-TV/GTfSvgfVwIFUGoASU7xm38opIBLjLMf1HeNTW07U=";
in
  rustPlatform.buildRustPackage rec {
    pname = "halley";
    version = "0.5.0";

    src = ../src;

    # Smithay is the only git dep; the rest come from crates.io via Cargo.lock.
    cargoLock = {
      lockFile = ../src/Cargo.lock;
      outputHashes = {
        "smithay-0.7.0" = smithayHash;
      };
    };

    nativeBuildInputs = [
      pkg-config
      makeWrapper
      # input-sys and libseat generate bindings via bindgen -> libclang.
      rustPlatform.bindgenHook
    ];

    buildInputs = [
      wayland # smithay's use_system_lib
      libxkbcommon
      libinput # backend_libinput
      seatd # backend_session_libseat (provides libseat)
      udev # backend_udev
      libgbm # backend_gbm
      libdrm # backend_drm
      libglvnd # backend_egl / renderer_gl
      pixman
      dbus # IPC / portal
      pipewire # libspa-sys / portal screencast
    ];

    # Tests require a live Wayland display + system fonts (panic with
    # NoWaylandLib / "no default font found") -> won't run in the sandbox.
    doCheck = false;

    # EGL / wayland-client link args (mirrors niri's pkgs/by-name/ni/niri):
    # smithay loads libEGL (via libglvnd) and libwayland-client through dlopen.
    # Forcing them into DT_NEEDED (resolved via the binary's RPATH, which Nix
    # fills from buildInputs) means Halley finds EGL/wayland WITHOUT an
    # LD_LIBRARY_PATH. We don't set LD_LIBRARY_PATH on the `halley` binary
    # because it's the compositor: everything it launches (terminal, browsers,
    # games) inherits its environment, and a stray LD_LIBRARY_PATH would mix
    # Nix libs with the system's and break GLX/wayland in those children.
    # libgbm/libdrm/libinput/libxkbcommon already enter as normal DT_NEEDED
    # via pkg-config and are covered by the same RPATH.
    RUSTFLAGS = lib.concatStringsSep " " (
      map (arg: "-C link-arg=" + arg) [
        "-Wl,--push-state,--no-as-needed"
        "-lEGL"
        "-lwayland-client"
        "-Wl,--pop-state"
      ]
    );

    # Put xwayland-satellite on the compositor's PATH: Halley launches it by
    # name (Command::new("xwayland-satellite")) for X11 app support.
    postFixup = ''
      wrapProgram $out/bin/halley \
        --prefix PATH : ${lib.makeBinPath [xwayland-satellite]}

      # halleyctl and the portal backend also dlopen EGL/GL/wayland. They're
      # safe to wrap with LD_LIBRARY_PATH (they don't spawn games / children
      # of your session).
      for bin in halleyctl xdg-desktop-portal-halley; do
        wrapProgram $out/bin/$bin \
          --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [
        libglvnd
        wayland
        libxkbcommon
        libgbm
        libdrm
        pipewire
      ]}
      done
    '';

    # Installs the Wayland session, portal metadata, systemd user units and
    # D-Bus service. We patch the upstream /usr/bin/* paths to land inside
    # the Nix store. We also embed the dbus-run-session workaround for
    # no-systemd distros (Void/runit) into halley-session, identical to
    # what ./void/halley-session does for the xbps package.
    postInstall = ''
      install -Dm755 packaging/wayland-sessions/halley-session $out/bin/halley-session
      substituteInPlace $out/bin/halley-session \
        --replace-fail /usr/bin/halley $out/bin/halley

      substituteInPlace $out/bin/halley-session \
        --replace-fail 'export HALLEY_WL_BACKEND=tty' \
        'if [ -z "''${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
    exec ${dbus}/bin/dbus-run-session -- "$0" "$@"
  fi
  export HALLEY_WL_BACKEND=tty'

      install -Dm644 packaging/wayland-sessions/halley.desktop \
        $out/share/wayland-sessions/halley.desktop
      substituteInPlace $out/share/wayland-sessions/halley.desktop \
        --replace-fail /usr/bin/halley-session $out/bin/halley-session

      install -Dm644 packaging/xdg-desktop-portal/portals/halley.portal \
        $out/share/xdg-desktop-portal/portals/halley.portal

      install -Dm644 packaging/xdg-desktop-portal/halley-portals.conf \
        $out/share/xdg-desktop-portal/halley-portals.conf

      install -Dm644 packaging/systemd-user/halley.service \
        $out/lib/systemd/user/halley.service
      install -Dm644 packaging/systemd-user/halley-shutdown.target \
        $out/lib/systemd/user/halley-shutdown.target
      substituteInPlace $out/lib/systemd/user/halley.service \
        --replace-fail /usr/bin/halley $out/bin/halley

      install -Dm644 packaging/dbus-1/services/org.freedesktop.impl.portal.desktop.halley.service \
        $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.halley.service
      substituteInPlace $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.halley.service \
        --replace-fail /usr/bin/xdg-desktop-portal-halley $out/bin/xdg-desktop-portal-halley
    '';

    # Used by HM's services.displayManager.sessionPackages / xdg.portal
    # machinery; must match the .desktop name (share/wayland-sessions/halley.desktop).
    passthru.providedSessions = ["halley"];

    meta = {
      description = "Spatial Wayland compositor (patched fork with fullscreen-layer, cursor-anim and idle-tick fixes)";
      homepage = "https://github.com/mikuri12/halley";
      license = lib.licenses.gpl3Only;
      mainProgram = "halley";
      platforms = lib.platforms.linux;
    };
  }
