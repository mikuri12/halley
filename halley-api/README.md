# `halley-api`

`halley-api` is the supported Rust SDK for building launchers, panels,
automation, and other tools against the Halley compositor. It provides typed
commands and queries, semantic IDs and state types, structured errors, and
sequenced subscriptions with an initial snapshot.

```rust
use halley_api::{Client, EventTopic, NodeSelector};

fn main() -> halley_api::Result<()> {
    let client = Client::connect()?;
    client.focus_node(Some(NodeSelector::Latest), None)?;

    let mut subscription = client.subscribe([EventTopic::Nodes])?;
    println!("{} nodes already exist", subscription.initial.nodes.len());
    for event in &mut subscription.events {
        println!("{:?}", event?);
    }
    Ok(())
}
```

See [`docs/api.md`](../docs/api.md) for the contract, operation list, event
semantics, and integration guidance.
