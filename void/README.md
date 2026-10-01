# Building the Void Linux package

Two installation methods available:

1. **Prebuilt binaries** — fast, no Rust/Cargo needed
2. **Compile from source** (`template`) — full build from scratch

## Option 1: Prebuilt binaries (recommended)

Downloads binaries built inside a Void glibc container by GitHub Actions.

Grab the `template` asset **from the release**, not `void/template-prebuilt`
from this repo — the in-repo file is a placeholder stub (`@TAG@`,
`@CHECKSUM@`) that Actions fills in at release time.

```sh
git clone --depth=1 https://github.com/void-linux/void-packages.git
cd void-packages
./xbps-src binary-bootstrap

mkdir -p srcpkgs/halley
curl -L -o srcpkgs/halley/template \
  https://github.com/mikuri12/halley/releases/download/v0.5.0-mikuri.2/template

./xbps-src pkg halley
doas xbps-install --repository hostdir/binpkgs halley   # or sudo
```

**No build dependencies required**, and no checksum to paste — the release
template ships with it already filled in.

See [README-PREBUILT.md](README-PREBUILT.md) for the full workflow (publishing
releases, why the build runs in a Void container).

## Option 2: Compile from source

```sh
git clone --depth=1 https://github.com/void-linux/void-packages.git
cd void-packages
./xbps-src binary-bootstrap

# Drop only the template (everything else is fetched by xbps-src):
mkdir -p srcpkgs/halley
curl -L -o srcpkgs/halley/template https://raw.githubusercontent.com/mikuri12/halley/main/void/template

./xbps-src pkg halley
doas xbps-install --repository hostdir/binpkgs halley   # or sudo
```

After install, log out and pick **Halley** from the Noctalia/ly/SDDM menu.
The `.desktop` points to `/usr/bin/halley-session`, which already carries
the `dbus-run-session` guard for Void runit (no session D-Bus by default).

## What the package installs

All of this comes from the build, no manual steps:

- `/usr/bin/halley`, `/usr/bin/halleyctl`, `/usr/bin/xdg-desktop-portal-halley`
- `/usr/bin/halley-session` — the wrapper with the `dbus-run-session` guard
- `/usr/share/wayland-sessions/halley.desktop`
- `/usr/share/xdg-desktop-portal/portals/halley.portal`
- `/usr/share/xdg-desktop-portal/halley-portals.conf`
- `/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.halley.service`

The portal backend is registered through the metadata + D-Bus service; no
`/etc/xdg-desktop-portal/portals.conf` is touched by the package.

## Build-time dependencies

Only Option 2 (source) pulls these: `pkg-config`, `rust`, `cargo`,
`clang18-devel`, `libclang18`, `wayland-devel`, `libxkbcommon-devel`,
`libinput-devel`, `libseat-devel`, `libudev-devel`, `libgbm-devel`,
`libdrm-devel`, `libglvnd-devel`, `pixman-devel`, `dbus-devel`,
`pipewire-devel`.

`clang18-devel` + `libclang18` are needed because `input-sys` and `libseat`
generate bindings via `bindgen`.

Option 1 (prebuilt) pulls **none** of them.

## Runtime dependencies

Declared explicitly via `depends=`:

- `xwayland-satellite` — Halley launches it by name for X11 app support.
- `dbus` — provides `dbus-run-session`, used by the `halley-session` guard.
- `seatd` — libseat backend for DRM VT handover. Make sure your user is
  in the `_seatd` group (or that the `seatd` runit service is enabled).

The shared libraries (wayland, libxkbcommon, libinput, libseat, libudev,
libgbm, libdrm, libglvnd, pixman, pipewire) are *not* listed by hand:
`xbps-src` scans the built ELFs and records them as `shlib-requires`, so
`xbps-install` pulls them in automatically.

## Notes

- `archs`: `x86_64*` for the source template (musl untested but plausible —
  Halley is pure Rust). The prebuilt template is `x86_64` with no `*`, since
  those binaries are glibc-linked and won't run on musl.
- **Never move a published tag.** GitHub regenerates the archive when a tag
  is repointed, so the SHA256 changes and the build breaks with a checksum
  mismatch. Cut a new tag instead.
- The source template pins `refs/tags/v0.5.0-mikuri.1.tar.gz`. To re-pin,
  push a new tag and recompute:
  `curl -sL https://github.com/mikuri12/halley/archive/refs/tags/<newtag>.tar.gz | sha256sum`.
- The prebuilt template needs no manual checksum: GitHub Actions injects it
  when publishing the release.
