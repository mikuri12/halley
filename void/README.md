# Building the Void Linux package

The xbps-src `template` lives in this folder. The wrapper `halley-session`
ships the workaround for the "no signal" bug on Void: the upstream one
assumes systemd-logind's session D-Bus, which doesn't exist under runit.
When `DBUS_SESSION_BUS_ADDRESS` is unset, this wrapper re-execs itself
under `dbus-run-session` so Halley actually gets a session bus and doesn't
hang in `dbus-update-activation-environment` -> `do_wait`.

## How to build & install

```sh
git clone https://github.com/void-linux/void-packages.git
cd void-packages

mkdir -p srcpkgs/halley
cp ../void/template            srcpkgs/halley/template
cp ../void/halley-session      srcpkgs/halley/
cp ../void/halley.desktop      srcpkgs/halley/
cp ../void/halley.portal       srcpkgs/halley/
cp ../void/halley-portals.conf srcpkgs/halley/

./xbps-src pkg halley
sudo xbps-install --repository hostdir/binpkgs halley
```

After install, log out and pick **Halley** from the Noctalia/ly/SDDM menu.
The `.desktop` points to `/usr/bin/halley-session`, which already carries
the `dbus-run-session` fix for Void runit (no session D-Bus by default).

## Notes

- `archs="x86_64*"`: the fork has only been tested on glibc x86_64. musl
  should work in principle (Halley is pure Rust) but is untested.
- `depends`: `xwayland-satellite` (X11 app support), `dbus` (provides
  `dbus-run-session`, used by the `halley-session` guard), `seatd`
  (libseat backend for DRM vt handover).
- `clang18-devel` + `libclang18` are needed because `input-sys` and
  `libseat` generate bindings via `bindgen` at build time.
