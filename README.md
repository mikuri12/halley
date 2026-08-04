# Halley — Mikuri build

This is a patched, source-only checkout of the
[Halley](https://github.com/saltnpepper97/halley) Wayland compositor (upstream
tag `v0.5.0`) with a set of local fixes applied on top.

The goal of this folder is to let **anyone** clone it, read the patches, and
build the compositor with `cargo` — no Nix, no per-machine hacks. The code is
already patched in [`src/`](./src/); the raw patches also live under
[`patches/`](./patches/) for reference / rebasing onto a newer upstream.

## Layout

```
halleymikuri/
├── src/                 # upstream v0.5.0 with the fixes already applied
│   ├── crates/          #   Cargo workspace (halley-wl is the main crate)
│   ├── packaging/       #   wayland-sessions .desktop, systemd-user units,
│   │                    #   xdg-desktop-portal config, dbus services
│   └── Cargo.toml       #   workspace manifest
├── patches/
│   ├── halley-fixes.patch                       # the canonical patch (already applied to src/)
│   ├── halley-fixes.patch.bak-previo-cursor     # earlier revision (before the cursor fix)
│   └── halley-fixes.patch.bak-previo-idle-cpu   # earlier revision (before the idle/cpu fix)
├── void/                # xbps-src template + assets for the Void Linux package
│   ├── template         #   xbps-src template (cargo build, installs bin + .desktop + portal)
│   ├── halley-session   #   wrapper with the dbus-run-session guard for runit/no-systemd
│   ├── halley.desktop   #   Wayland-sessions .desktop pointing at /usr/bin/halley-session
│   ├── halley.portal    #   xdg-desktop-portal backend metadata
│   ├── halley-portals.conf  # portal routing (ScreenCast/Screenshot -> halley backend)
│   └── README.md        #   build & install instructions for Void
└── nix/                 # Nix flake exposing the package + a HM configuration
    ├── flake.nix        #   packages.halley + homeConfigurations.mikuri
    └── halley.nix       #   Rust derivation (callPackage-able)
```

> The two `.bak-previo-*` files are kept for archaeological reasons. To rebuild
> `src/` from the upstream tag you only need `halley-fixes.patch`. If you ever
> rebase onto a newer upstream, apply that patch and resolve the conflicts; the
> `.bak` files are not part of that flow.

## What the patch fixes

`halley-fixes.patch` bundles six independent fixes, each explained in a comment
at the hunk it touches. Summary:

### 1. Direct-scanout no longer disabled by pending frame-callbacks
`crates/halley-wl/src/backend/tty/drm.rs`

A fullscreen client **always** has pending frame-callbacks, so the old gate
that turned off direct-scanout whenever callbacks were pending made the scanout
path oscillate to the composited (GL) path on every frame. Result: flicker and
a noticeable fps drop in fullscreen games. The patch drops the
`fullscreen_needs_paced_frames` gate: the direct-scanout path is already paced
by page-flip + presentation feedback further down, so the gate was redundant
and harmful.

### 2. Fullscreen apps cover the Top/Overlay layer-shell surfaces
`crates/halley-wl/src/compositor/fullscreen/system.rs`,
`crates/halley-wl/src/render/frame/draw.rs`,
`crates/halley-wl/src/render/frame/scene.rs`,
`crates/halley-wl/src/input/pointer/focus/surface.rs`

Halley has no dedicated fullscreen render route, so the Top/Overlay
layer-shell surfaces (bar, notifications, launcher) were drawn **after** the
window stack and always sat on top of a fullscreen app. The patch:

- Adds `current_monitor_has_settled_fullscreen(st, now)` — true when the
  current render monitor has a fullscreen node whose enter/exit animation is
  idle. Gating on "animation idle" means the bar re-appears while the
  fullscreen zooms **out** on exit, instead of popping back only at the end.
- Adds a `suppress_top_overlay_layers` flag to `SceneCollections` that the
  frame builder consults to skip drawing `layer_top_elements` and
  `layer_overlay_elements`.
- Makes the **hit-test** use the same predicate, otherwise the layers stayed
  invisible but clickable: a click "inside" the fullscreen app was swallowed
  by a layer the user couldn't see.
- Reverses the layer-shell placement list before the stable layer sort, so a
  fullscreen surface mapped *before* a same-layer panel (e.g. a panel's own
  click-shield) doesn't win the hit-test over the panel above it.

### 3. `send_pending_configure()` instead of unconditional `send_configure()`
`crates/halley-wl/src/compositor/fullscreen/system.rs`

A client already in fullscreen that re-requests `set_fullscreen`
(Chromium/Electron do this on tab/video switches) used to receive an
**identical** configure event with Fullscreen reasserted. The browser read
that as "you just entered fullscreen" and re-shewed the enter-fullscreen toast,
which then never dismissed. `send_pending_configure()` is Smithay's own dedup
— a no-op when the pending state equals the already-sent one. Same pattern
upstream already uses in `compositor/overlap/system/resolve.rs`.

### 4. Animated XCursor support (multi-frame)
`crates/halley-wl/src/render/cursor_theme.rs`,
`crates/halley-wl/src/render/cursor.rs`,
`crates/halley-wl/src/render/frame/draw.rs`,
`crates/halley-wl/src/frame_loop/activity.rs`,
`crates/halley-wl/src/portal/mod.rs`

Upstream only kept the **first** XCursor frame, so every animated cursor
theme was static. The patch:

- Adds a `FrameData { pixels_bgra, delay_ms }` struct and replaces the single
  `pixels_bgra` field of `SoftwareCursorSprite` with `frames: Vec<FrameData>`.
- `SoftwareCursorSprite::frame_at(elapsed_ms)` picks the right frame by
  walking the cumulative delays (single-frame sprites short-circuit).
- `CursorManager` now tracks an animation `started_at` + `cycle_ms` and
  resets them only when the named icon **actually changes** (not every render
  frame).
- Forces an output redraw while an animated named cursor is active, so the
  blit advances frames by elapsed time without needing a dedicated timer.
- The screencast portal takes a static first-frame snapshot for its cursor
  metadata — the client gets a fixed image, as before.

### 5. Adaptive idle tick + display-fd in calloop
`crates/halley-wl/src/backend/tty/mod.rs`

At 60 Hz the master calloop timer was the heartbeat that drove the whole
compositor, so it re-armed on every iteration and the process never went idle.
The patch:

- Adds an `IDLE_TICK_MS = 100` const and a `busy` predicate; when nothing
  pending needs a fast tick (no redraws queued, no spawned children, no
  pending frame callbacks, no DPMS-pending frames, no config watch, no
  active animation, not in the first 6 s of boot) the timer is re-armed to
  the slow `IDLE_TICK_MS` instead of `frame_interval`.
- Registers the **display fd** itself in calloop (`Interest::READ`,
  `Mode::Level`) so a client commit wakes the loop on its own. Without this,
  the only thing that serviced clients was the 16.7 ms timer, which is exactly
  what kept the compositor polling permanently at idle.

### 6. CursorManager carried into the direct-scanout cursor path
`crates/halley-wl/src/backend/tty/drm.rs`, `render/cursor.rs`

Side-effect of fix #4: the direct-scanout cursor path used to read the global
cache directly, bypassing the animation state. The patch threads a
`&mut CursorManager` down through `queue_tty_drm_frame` →
`render_tty_direct_elements` → `direct_scanout_cursor_elements` so the sprite
is resolved via `cursor_manager.sprite_with_fallback(...)`, which keeps the
`started_at`/`cycle_ms` authoritative also when the cursor goes out via the
DRM HW plane (where `draw_cursor_layer` does not run).

## Build from source

### Native toolchain

You need a reasonably recent Rust (stable is fine; the workspace builds with
`resolver = "2"`). You also need `pkg-config` and the C libraries that
smithay links against. On a Debian/Ubuntu-ish system that's roughly:

```sh
sudo apt install pkg-config build-essential \
                 libwayland-dev libxkbcommon-dev libinput-dev libseat-dev \
                 libudev-dev libgbm-dev libdrm-dev libglvnd-dev \
                 libpixman-1-dev libdbus-1-dev libpipewire-0.3-dev
```

On Void Linux (what this config actually targets):

```sh
sudo xbps-install -S pkg-config clang-devel wayland-devel libxkbcommon-devel \
                   libinput-devel libseat-devel libudev-devel libgbm-devel \
                   libdrm-devel libglvnd-devel pixman-devel dbus-devel \
                   pipewire-devel
```

> `input-sys` and `libseat` generate bindings with `bindgen`, so `libclang`
> is needed at build time (the `clang-devel` / `libclang-dev` package).

Then:

```sh
cd src
cargo build --release
# Binaries land in:
#   target/release/halley
#   target/release/halleyctl
#   target/release/xdg-desktop-portal-halley
```

Tests are gated behind a live Wayland display and system fonts (they panic
with `NoWaylandLib` / "no default font found") and cannot run in a sandbox,
so the Nix derivation sets `doCheck = false`. You can run them manually on a
real session with `cargo test` if you want.

### EGL / wayland-client runtime resolution

smithay loads `libEGL` (via libglvnd) and `libwayland-client` through
`dlopen()`. On the Nix derivation these are forced in as `DT_NEEDED` via link
args so the compositor finds them without an `LD_LIBRARY_PATH`. If you build
natively against the system pkg-config, the system loader already resolves
them from the default search path and you don't need to do anything.

If your distro doesn't ship a default search path that covers them (rare),
set `LD_LIBRARY_PATH` to point at the directories that contain `libEGL.so`
and `libwayland-client.so` before launching. **Do not** bake that env into
the `halley` binary if you can avoid it — Halley is the compositor, so
everything it launches (terminal, games, browsers) inherits its environment,
and a stray `LD_LIBRARY_PATH` will mix libraries across distros and break GLX.

### X11 apps

Halley launches [xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite)
by name (`Command::new("xwayland-satellite)`) for X11 app support. Make sure
that binary is on `PATH` when you start Halley.

### Installing the session

The `packaging/` folder in `src/` already contains everything you need. The
upstream `halley-session` script hardcodes `/usr/bin/halley` and assumes a
session D-Bus. The Nix derivation in the parent repo rewrites both; for a
native install you either patch them yourself after install, or start
`halley` directly from your display manager's exec line.

Typical install into `/usr/local`:

```sh
sudo install -Dm755 target/release/halley             /usr/local/bin/halley
sudo install -Dm755 target/release/halleyctl         /usr/local/bin/halleyctl
sudo install -Dm755 target/release/xdg-desktop-portal-halley \
        /usr/local/bin/xdg-desktop-portal-halley
sudo install -Dm755 src/packaging/wayland-sessions/halley-session \
        /usr/local/bin/halley-session
sudo install -Dm644 src/packaging/wayland-sessions/halley.desktop \
        /usr/local/share/wayland-sessions/halley.desktop
sudo install -Dm644 src/packaging/xdg-desktop-portal/portals/halley.portal \
        /usr/local/share/xdg-desktop-portal/portals/halley.portal
sudo install -Dm644 src/packaging/xdg-desktop-portal/halley-portals.conf \
        /usr/local/share/xdg-desktop-portal/halley-portals.conf
sudo install -Dm644 src/packaging/dbus-1/services/org.freedesktop.impl.portal.desktop.halley.service \
        /usr/local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.halley.service
```

After that you'll probably want to edit
`/usr/local/share/wayland-sessions/halley.desktop` to point at
`/usr/local/bin/halley-session`, and `/usr/local/bin/halley-session` to
`/usr/local/bin/halley`, so the paths match your prefix.

## Install on Void Linux (xbps package)

This repo ships an `xbps-src` template under [`void/`](./void/) that builds
the compositor as a normal Void package and installs the patched
`halley-session` wrapper (with the `dbus-run-session` fix for the "no
signal" bug under runit — see `void/halley-session` for the rationale).

### Build & install

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
the `dbus-run-session` guard for Void runit (no session D-Bus by default).

### Runtime dependencies

`depends=` in the template pulls in:

- `xwayland-satellite` — Halley launches it by name for X11 app support.
- `dbus` — provides `dbus-run-session`, used by the `halley-session`
  guard when there's no session bus.
- `seatd` — libseat backend for DRM VT handover. Make sure your user is
  in the `_seatd` group (or that the `seatd` runit service is enabled).

See [`void/README.md`](./void/README.md) for the build-time dependencies
(`clang18-devel`, `wayland-devel`, etc.) and notes.

## Install on NixOS / Home Manager (flake)

This repo exposes a flake under [`nix/`](./nix/) with:

- `packages.${system}.halley` — the Rust derivation (callable from any
  other flake as an input).
- `packages.${system}.default` — alias of `halley`.
- `homeConfigurations.mikuri` — a ready-to-use Home Manager standalone
  configuration that installs the package and exposes the Wayland
  session + portal metadata.

### Use it from your existing flake

Add this repo as an input and import the package:

```nix
# your-flake.nix
inputs.halley.url = "github:mikuri12/halley";
# ...
environment.systemPackages = [ inputs.halley.packages.${system}.halley ];
# or, on Home Manager:
home.packages = [ inputs.halley.packages.${system}.halley ];
```

The `halley` derivation ships `passthru.providedSessions = [ "halley" ]`,
so on NixOS you can also register it with the display manager:

```nix
services.displayManager.sessionPackages = [ inputs.halley.packages.${system}.halley ];
xdg.portal.extraPortals = [ inputs.halley.packages.${system}.halley ];
```

### Apply the ready-made HM configuration

If you just want to try it on a non-NixOS distro (Void, Arch, …) with
Home Manager standalone:

```sh
home-manager switch --flake github:mikuri12/halley#mikuri
```

This installs `halley`, `halleyctl`, `xdg-desktop-portal-halley` into the
`mikuri` user profile and writes the Wayland `.desktop` + portal metadata
into `~/.local/share/`. The `halley-session` wrapper shipped with the Nix
package already contains the `dbus-run-session` guard (same as the Void
one), so it works on systems without a session D-Bus.

> Note: `home.username` is hardcoded to `mikuri` in the example config.
> Fork the repo or override the module with your own username.
> `home.stateVersion = "25.05"`.

### Notes on the Nix build

- `src` is the local `../src` tree (already patched), not a
  `fetchFromGitHub`. This means `nix build` re-reads the patch state from
  disk — no need to re-apply anything.
- `-C target-cpu=native` and fat-LTO are **disabled** (those were the
  original packager's per-machine tuning). Only the EGL/wayland-client
  link args stay, because without them Halley can't dlopen EGL and falls
  back to polluting child envs via `LD_LIBRARY_PATH` (which breaks GLX
  in games/browsers launched from the compositor).
- HD530-specific `MESA_LOADER_DRIVER_OVERRIDE=iris` / `INTEL_DEBUG=no32`
  env vars are **not** set. Add them yourself in your HM config if your
  GPU needs the same settling the original developer's HD530 did.

## Re-applying the patch onto a newer upstream

```sh
git clone --branch v0.6.0 https://github.com/saltnpepper97/halley new-halley
cd new-halley
git apply /path/to/halleymikuri/patches/halley-fixes.patch
```

If a hunk no longer applies, look at the corresponding section in this README
— the *intent* of each fix is documented, which is usually enough to resolve
the conflict by hand. The two `.bak-previo-*` patches are **not** meant to be
re-applied; they're historical snapshots.
