use halley_api::{Client, NodeSelector};

fn main() -> halley_api::Result<()> {
    let client = Client::connect()?;
    println!(
        "connected to Halley {}",
        client.server_info().compositor_version
    );

    for output in client.outputs()? {
        println!(
            "{} at {}, {}",
            output.name, output.offset_x, output.offset_y
        );
    }

    client.focus_node(Some(NodeSelector::Latest), None)
}
