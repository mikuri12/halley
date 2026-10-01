use halley_api::{Client, EventTopic};

fn main() -> halley_api::Result<()> {
    let client = Client::connect()?;
    let mut subscription = client.subscribe([
        EventTopic::Outputs,
        EventTopic::Nodes,
        EventTopic::Clusters,
        EventTopic::Config,
    ])?;

    println!(
        "snapshot {}: {} outputs, {} nodes, {} clusters",
        subscription.initial.sequence,
        subscription.initial.outputs.len(),
        subscription.initial.nodes.len(),
        subscription.initial.clusters.len()
    );
    for event in &mut subscription.events {
        println!("{:?}", event?);
    }
    Ok(())
}
