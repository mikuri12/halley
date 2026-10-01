pub(crate) mod camera;
pub(crate) mod maximize;
pub(crate) mod window;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PresentationWorkspace {
    Field,
    Cluster(halley_core::cluster::ClusterId),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PresentationScope {
    pub(crate) output: String,
    pub(crate) workspace: PresentationWorkspace,
}

impl PresentationScope {
    pub(crate) fn new(output: impl Into<String>, workspace: PresentationWorkspace) -> Self {
        Self {
            output: output.into(),
            workspace,
        }
    }
}

pub(crate) fn workspace_for_surface(
    clusters: &crate::clusters::ClusterSystem,
    nodes: &crate::nodes::NodesState,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) -> PresentationWorkspace {
    nodes
        .id_for_surface(surface)
        .and_then(|node| clusters.cluster_for_member(node))
        .map_or(PresentationWorkspace::Field, PresentationWorkspace::Cluster)
}

pub(crate) fn active_workspace_on_output(
    clusters: &crate::clusters::ClusterSystem,
    output: &str,
    now: std::time::Duration,
) -> PresentationWorkspace {
    clusters
        .active_on(output)
        .or_else(|| clusters.transition_cluster_on(output, now))
        .map_or(PresentationWorkspace::Field, PresentationWorkspace::Cluster)
}

pub(crate) fn surface_workspace_is_active(
    clusters: &crate::clusters::ClusterSystem,
    nodes: &crate::nodes::NodesState,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    output: &str,
    now: std::time::Duration,
) -> bool {
    workspace_for_surface(clusters, nodes, surface)
        == active_workspace_on_output(clusters, output, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_and_cluster_presentations_have_independent_output_scopes() {
        let cluster = halley_core::cluster::ClusterId::new(7);
        let field = PresentationScope::new("DP-1", PresentationWorkspace::Field);
        let clustered = PresentationScope::new("DP-1", PresentationWorkspace::Cluster(cluster));
        let other_output = PresentationScope::new("DP-2", PresentationWorkspace::Field);

        assert_ne!(field, clustered);
        assert_ne!(field, other_output);
        assert_eq!(
            clustered,
            PresentationScope::new("DP-1", PresentationWorkspace::Cluster(cluster))
        );
    }
}
