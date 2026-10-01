use crate::{
    ClusterDraftId, ClusterDraftState, ClusterSummary, Error, ErrorKind, NodeId, NodeInfo,
    OutputInfo, Result, Snapshot,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventTopic {
    Outputs,
    Nodes,
    NodeGeometry,
    Clusters,
    Config,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    OutputAdded {
        sequence: u64,
        output: OutputInfo,
    },
    OutputChanged {
        sequence: u64,
        output: OutputInfo,
    },
    OutputRemoved {
        sequence: u64,
        name: String,
    },
    NodeAdded {
        sequence: u64,
        node: NodeInfo,
    },
    NodeChanged {
        sequence: u64,
        node: NodeInfo,
    },
    NodeGeometryChanged {
        sequence: u64,
        id: NodeId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    NodeRemoved {
        sequence: u64,
        id: NodeId,
    },
    ClusterAdded {
        sequence: u64,
        cluster: ClusterSummary,
    },
    ClusterChanged {
        sequence: u64,
        cluster: ClusterSummary,
    },
    ClusterRemoved {
        sequence: u64,
        id: crate::ClusterId,
    },
    ConfigReloaded {
        sequence: u64,
        accepted: bool,
    },
    ClusterDraftChanged {
        sequence: u64,
        id: ClusterDraftId,
        state: ClusterDraftState,
        message: Option<String>,
    },
}

impl Event {
    pub fn sequence(&self) -> u64 {
        match self {
            Self::OutputAdded { sequence, .. }
            | Self::OutputChanged { sequence, .. }
            | Self::OutputRemoved { sequence, .. }
            | Self::NodeAdded { sequence, .. }
            | Self::NodeChanged { sequence, .. }
            | Self::NodeGeometryChanged { sequence, .. }
            | Self::NodeRemoved { sequence, .. }
            | Self::ClusterAdded { sequence, .. }
            | Self::ClusterChanged { sequence, .. }
            | Self::ClusterRemoved { sequence, .. }
            | Self::ConfigReloaded { sequence, .. }
            | Self::ClusterDraftChanged { sequence, .. } => *sequence,
        }
    }
}

pub struct Subscription {
    pub initial: Snapshot,
    pub events: EventStream,
}

pub struct EventStream {
    pub(crate) connection: halley_ipc::Connection,
    pub(crate) last_sequence: u64,
}

impl EventStream {
    pub fn next_event(&mut self) -> Result<Event> {
        let envelope = self.connection.receive()?;
        if !envelope.fds.is_empty() {
            return Err(Error::new(
                ErrorKind::Protocol,
                "event carried unexpected file descriptors",
            ));
        }
        let halley_ipc::Response::Event(event) = envelope.response else {
            return Err(Error::new(ErrorKind::Protocol, "expected an event frame"));
        };
        let event = convert_event(event);
        let sequence = event.sequence();
        if sequence != self.last_sequence.saturating_add(1) {
            return Err(Error::new(
                ErrorKind::Protocol,
                format!(
                    "event sequence gap: expected {}, received {sequence}; reconnect for a fresh snapshot",
                    self.last_sequence.saturating_add(1)
                ),
            ));
        }
        self.last_sequence = sequence;
        Ok(event)
    }
}

impl Iterator for EventStream {
    type Item = Result<Event>;
    fn next(&mut self) -> Option<Self::Item> {
        Some(self.next_event())
    }
}

fn convert_event(v: halley_ipc::ApiEvent) -> Event {
    match v {
        halley_ipc::ApiEvent::OutputAdded { sequence, output } => Event::OutputAdded {
            sequence,
            output: output.into(),
        },
        halley_ipc::ApiEvent::OutputChanged { sequence, output } => Event::OutputChanged {
            sequence,
            output: output.into(),
        },
        halley_ipc::ApiEvent::OutputRemoved { sequence, name } => {
            Event::OutputRemoved { sequence, name }
        }
        halley_ipc::ApiEvent::NodeAdded { sequence, node } => Event::NodeAdded {
            sequence,
            node: node.into(),
        },
        halley_ipc::ApiEvent::NodeChanged { sequence, node } => Event::NodeChanged {
            sequence,
            node: node.into(),
        },
        halley_ipc::ApiEvent::NodeGeometryChanged {
            sequence,
            id,
            pos_x,
            pos_y,
            width,
            height,
        } => Event::NodeGeometryChanged {
            sequence,
            id: NodeId::new(id),
            x: pos_x,
            y: pos_y,
            width,
            height,
        },
        halley_ipc::ApiEvent::NodeRemoved { sequence, id } => Event::NodeRemoved {
            sequence,
            id: NodeId::new(id),
        },
        halley_ipc::ApiEvent::ClusterAdded { sequence, cluster } => Event::ClusterAdded {
            sequence,
            cluster: cluster.into(),
        },
        halley_ipc::ApiEvent::ClusterChanged { sequence, cluster } => Event::ClusterChanged {
            sequence,
            cluster: cluster.into(),
        },
        halley_ipc::ApiEvent::ClusterRemoved { sequence, id } => Event::ClusterRemoved {
            sequence,
            id: crate::ClusterId::new(id),
        },
        halley_ipc::ApiEvent::ConfigReloaded { sequence, accepted } => {
            Event::ConfigReloaded { sequence, accepted }
        }
        halley_ipc::ApiEvent::ClusterDraftChanged {
            sequence,
            id,
            state,
            message,
        } => Event::ClusterDraftChanged {
            sequence,
            id: ClusterDraftId::new(id),
            state: match state {
                halley_ipc::ClusterDraftState::Started => ClusterDraftState::Started,
                halley_ipc::ClusterDraftState::AwaitingName => ClusterDraftState::AwaitingName,
                halley_ipc::ClusterDraftState::Launching => ClusterDraftState::Launching,
                halley_ipc::ClusterDraftState::Completed => ClusterDraftState::Completed,
                halley_ipc::ClusterDraftState::Cancelled => ClusterDraftState::Cancelled,
                halley_ipc::ClusterDraftState::Failed => ClusterDraftState::Failed,
            },
            message,
        },
    }
}
