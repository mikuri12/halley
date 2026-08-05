# Building the Void Linux package

The xbps-src `template` here builds the compositor as a normal Void
package, downloading the source directly from this fork on GitHub via
`distfiles`. You don't copy any local files besides the template itself.

## How to build & install

```sh
git clone https://github.com/void-linux/void-packages.git
cd void-packages

# Drop only the template (everything else is fetched by xbps-src):
mkdir -p srcpkgs/halley
curl -L -o srcpkgs/halley/template https://raw.githubusercontent.com/mikuri12/halley/main/void/template

./xbps-src pkg halley
sudo xbps-install --repository hostdir/binpkgs halley
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

The template pulls `pkg-config`, `rust`, `cargo`, `clang18-devel`,
`libclang18`, `wayland-devel`, `libxkbcommon-devel`, `libinput-devel`,
`libseat-devel`, `libudev-devel`, `libgbm-devel`, `libdrm-devel`,
`libglvnd-devel`, `pixman-devel`, `dbus-devel`, `pipewire-devel`.

`clang18-devel` + `libclang18` are needed because `input-sys` and `libseat`
generate bindings via `bindgen`.

## Runtime dependencies

Pulled in automatically via `depends=`:

- `xwayland-satellite` — Halley launches it by name for X11 app support.
- `dbus` — provides `dbus-run-session`, used by the `halley-session` guard.
- `seatd` — libseat backend for DRM VT handover. Make sure your user is
  in the `_seatd` group (or that the `seatd` runit service is enabled).

## Notes

- `archs="x86_64*"`: the fork has only been tested on glibc x86_64. musl
  should work in principle (Halley is pure Rust) but is untested.
- The `checksum` in the template is the SHA256 of the pinned release
  tarball `refs/tags/v0.5.0-mikuri.1.tar.gz`. Pinning to a tag (not to
  `refs/heads/main`) keeps the checksum stable across future commits.
- To re-pin to a new release, push the new tag and recompute:
  `curl -sL https://github.com/mikuri12/halley/archive/refs/tags/<newtag>.tar.gz | sha256sum`.
