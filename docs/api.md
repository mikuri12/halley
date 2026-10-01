# Halley compositor API

`halley-api` is Halley's supported integration boundary for launchers, panels,
automation, testing tools, and desktop components. It is the layer external
programs should build against. `halley-ipc` is the private wire codec used by
the SDK and compositor; its postcard representation is not a public contract.

## Add the SDK

From a sibling project beside the active Halley checkout, use a path dependency:

```toml
[dependencies]
halley-api = { path = "../halley/halley-api" }
```

An application normally opens one `Client` and retains it. The client performs
an API-version handshake when it connects and keeps its command socket open:

```rust
use halley_api::{Client, NodeSelector};

fn main() -> halley_api::Result<()> {
    let client = Client::connect()?;
    println!("Halley {}", client.server_info().compositor_version);
    client.focus_node(Some(NodeSelector::Latest), None)
}
```

`Client::connect()` discovers the compositor socket beneath
`$XDG_RUNTIME_DIR/halley`. `Client::connect_to()` supports tests and custom
session wiring. `Client::connect_with()` additionally accepts read and write
timeouts.

`Client` is safe to share between threads. Commands on one client are
serialized over its persistent connection; a subscription owns a separate
connection so a blocking event reader never blocks commands.

## Supported operations

The version-1 semantic API includes:

| Area | Operations |
| --- | --- |
| Server | handshake, version and capability discovery, quit request |
| Outputs | list output modes and placement; set DPMS state |
| Nodes | list, inspect, focus, move, close, collapse, restore, and toggle |
| Clusters | list, inspect, open, activate a slot, cycle layout, finalize a draft |
| Bearings | query visibility; show, hide, or toggle |
| Configuration | discover the selected path and request a live reload |
| Events | output, node, node-geometry, cluster, draft, and config changes |

Selectors and IDs are semantic types such as `NodeId`, `NodeSelector`,
`ClusterId`, and `ClusterTarget`; callers do not construct wire requests.
Public state and event values implement Serde serialization and deserialization
for application persistence, logging, and JSON output.

## Subscriptions

A subscription returns a race-free initial snapshot and then ordered deltas:

```rust
use halley_api::{Client, Event, EventTopic};

fn watch() -> halley_api::Result<()> {
    let client = Client::connect()?;
    let mut subscription = client.subscribe([EventTopic::Nodes])?;
    render_full_state(&subscription.initial.nodes);

    for event in &mut subscription.events {
        match event? {
            Event::NodeAdded { node, .. } | Event::NodeChanged { node, .. } => {
                update_node(node)
            }
            Event::NodeRemoved { id, .. } => remove_node(id),
            _ => {}
        }
    }
    Ok(())
}

# fn render_full_state(_: &[halley_api::NodeInfo]) {}
# fn update_node(_: halley_api::NodeInfo) {}
# fn remove_node(_: halley_api::NodeId) {}
```

Every delta carries a monotonically increasing sequence number. The SDK checks
continuity and returns `ErrorKind::Protocol` on a gap; reconnect and rebuild
from the next initial snapshot in that case. The compositor uses a bounded
queue per subscriber and disconnects a consumer that cannot keep up, so a
stalled panel cannot consume unbounded compositor memory.

`EventTopic::NodeGeometry` is separate from `EventTopic::Nodes` because motion
can be high-volume. Subscribe only when live geometry is actually required.

## Cluster drafts

`Client::finalize_cluster_draft` is the integration point for launchers. A
draft contains selected running `NodeId`s plus application IDs and commands.
The compositor opens its naming UI, launches applications only after the user
confirms, stages matching Wayland and XWayland windows without exposing partial
state, and publishes draft lifecycle events. Unmatched applications time out
after 30 seconds and the completed membership is committed atomically.

## Errors and compatibility

All SDK operations return `halley_api::Result<T>`. Branch on `Error::kind()`;
messages are diagnostics for humans and are not stable parsing targets. Public
categories include `Connection`, `Protocol`, `InvalidRequest`, `NotFound`,
`Ambiguous`, `Unsupported`, `VersionMismatch`, `Busy`, and `Internal`.

`HALLEY_API_VERSION` versions the semantic contract. The handshake fails with
`VersionMismatch` when a compositor cannot provide that contract. Capability
strings allow optional extensions to be detected without guessing from the
compositor version. Changes to the private IPC codec do not by themselves
change the API version.

The workspace's `halleyctl` and `halley-lift` are reference consumers: neither
constructs low-level node or cluster wire messages.
