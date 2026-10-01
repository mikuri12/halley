# XWayland policy and conformance baseline

Halley's XWayland support is an in-tree compositor policy, not an upstreaming
patch set. The Smithay dependency stays on the repository's pinned URL and
revision. Halley adds only the state and decisions that must remain coherent
with its field, nodes, clusters, focus, and output model.

The normative references are the
[ICCCM](https://www.x.org/releases/current/doc/xorg-docs/icccm/icccm.pdf) and
[EWMH](https://specifications.freedesktop.org/wm/latest-single/).

## Ownership boundary

Smithay's XWM remains the sole owner of:

- `WM_S0`, substructure redirection, reparenting, and the X11 event stream;
- `_NET_CLIENT_LIST` and `_NET_CLIENT_LIST_STACKING` maintenance;
- XWayland surface association, focus protocol delivery, clipboard, and DND.

Halley's property-only X11 connection does not select root events or acquire a
manager selection. It publishes the narrower capability set Halley actually
implements, the support-window name, single-desktop geometry, active-window
state, per-window allowed actions and frame extents, ICCCM `WM_STATE`, and the
XKB repeat policy. Halley deliberately does not nominate a RandR primary
output; its internal fallback output is not an X11 desktop policy.

The client lists stay Smithay's because it already keeps them in step with the
real X stack. Halley's contribution is to make that stack agree with what is on
screen: the compositor's order is pushed down with
`X11Wm::update_stacking_order_downwards`, where `X11Wm::raise_window` alone
would only cover raise-to-top. That runs on focus changes, which is where map,
unmap, destroy and raise funnel, and again in the per-dispatch sweep beside
the position resync, which is the backstop for reorders with no focus change.
Both are guarded by a compare-before-send memo, because the call grabs the X
server for the whole walk.

This split is deliberate. The property connection never takes ownership of
RandR policy, and primary-output change notifications are observed without
being rewritten.

## Managed-window lifecycle

Each managed XID receives a monotonically increasing generation so a reused XID
cannot inherit state from a destroyed window. Its lifecycle is explicit:

- a new window starts Withdrawn;
- an admitted map enters Normal and receives `WM_STATE = NormalState`;
- an initial `WM_HINTS` IconicState request enters Halley's collapsed-node path;
- compositor or client minimize enters Iconic, sets EWMH hidden, and publishes
  `WM_STATE = IconicState`;
- restore clears hidden and returns to Normal;
- withdrawal deletes `WM_STATE`; destruction retires the generation.

The same transition helpers serve XWM requests, keybinds, IPC, and other
compositor-driven node operations. Focus changes update `_NET_ACTIVE_WINDOW`
only when the effective X11 target changes. A rejected stale activation request
sets demands-attention; successful focus and explicit client removal clear it.
The ICCCM urgency bit is bridged into the same attention state.

## Configure policy

`WM_NORMAL_HINTS` enforcement is isolated in `xwayland::xwm::configure` and is
shared by initial placement, client requests, tiling/cluster layouts, and
interactive resize. It handles:

- minimum and maximum sizes;
- the base-size arithmetic progression and resize increments;
- minimum and maximum aspect ratios relative to the base size;
- invalid ranges, non-positive increments, overflow, and impossible aspect
  combinations without panicking.

Halley owns the position and stack of ordinary field windows. Their client
position and restack requests are denied, with a synthetic `ConfigureNotify`
describing the effective compositor-owned geometry as ICCCM requires.
Transient coordinates are honored. Override-redirect clients remain
self-configuring and are mirrored into Halley's scene by their notify path.

Managed geometry has an explicit coordinate boundary: `Space` positions are
Field/source coordinates, while `X11Surface::geometry()` positions are X root
desktop coordinates. Halley maps the root-surface origin through the same live
presentation transform used for rendering and input, publishes it only after
geometry motion settles, and never copies a managed `ConfigureNotify` root
position back into `Space`. Native client size is kept unchanged under Field
zoom. Transient root positions are inverted into Field coordinates before
their compositor elements are placed. Owner-associated override-redirect
windows instead preserve the native-unit delta between their X root position
and the owner's last published X root origin, then inherit the owner's live
Field transform. This scales dropdown offsets exactly once and keeps them
attached while the camera is zoomed or panned. Ownerless override-redirect
windows keep using the absolute root-to-Field inversion.

Left/top interactive resize computes its anchor after hint constraint snapping,
so a terminal-size increment cannot move the opposite edge.

## Advertised EWMH subset

`_NET_SUPPORTED` is rewritten after Smithay initializes the XWM. It includes
active/client lists, the single-desktop read model, moveresize, maximize,
minimize/hidden, fullscreen, focused, demands-attention, taskbar/pager hints,
allowed actions, and frame extents.

`_NET_FRAME_EXTENTS` matters because Halley's border and titlebar live in the
compositor's scene, not in a reparented X frame: the geometry a client reads
back is its content area alone, so without the property its own root-coordinate
arithmetic is off by the decoration. The extents come from
`titlebar::frame_extents`, which derives them from the same three inputs as
`outer_size_for_client`, and are republished when a client toggles Motif
decorations or the decoration configuration is reloaded.

Halley does not advertise behavior it does not implement. In particular,
`_NET_CLOSE_WINDOW`/`_NET_WM_ACTION_CLOSE`, above/below layers, shading, and
sticky state remain absent. Adding one requires both the client-message/state
handler and compositor-side policy before adding its atom.

`_NET_REQUEST_FRAME_EXTENTS` is absent for the same reason. It arrives as a
root-window client message, and the root event stream belongs to Smithay, which
routes unrecognized messages to its unhandled branch. Supporting it would mean
either patching the pinned revision or giving the control connection its own
root event mask and event source.

## Validation

The pure lifecycle and configure policies are covered by unit tests, including
XID reuse, 32-bit timestamp wrap, initial/normal/iconic transitions, malformed
size hints, base-relative aspect math, and transient position policy.

For a runtime check, build the executable, start a nested session with the
inert example configuration, and use a disposable X11 client. Verify:

1. the support window names `Halley` and `_NET_SUPPORTED` contains only the
   implemented subset;
2. map/focus populate both client lists and `_NET_ACTIVE_WINDOW`, and
   `_NET_CLIENT_LIST_STACKING` follows raises and lowers;
3. `WM_STATE`, `_NET_WM_STATE`, `_NET_WM_ALLOWED_ACTIONS`, and
   `_NET_FRAME_EXTENTS` agree with what is drawn;
4. collapse produces Iconic + hidden + no active window, and restore produces
   Normal + focused + the client XID as active;
5. `xrandr --listmonitors` shows no compositor-appointed primary output;
6. after Field pan/zoom and a fullscreen cycle, X root geometry matches the
   settled visual root origin even when the stored Field position differs;
7. client exit empties the client/stack lists without protocol warning bursts.

Run the normal repository gates after the nested check:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo build -p halley
cargo clippy --workspace --all-targets -- -D warnings
```
