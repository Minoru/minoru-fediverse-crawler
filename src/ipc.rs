use serde::{Deserialize, Serialize};
use url::Host;

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum InstanceState {
    /// The instance is alive (it responded with a valid NodeInfo document).
    Alive,

    /// The instance responded with a temporary redirect (HTTP codes 302, 303, 307).
    Moving { to: Host },

    /// The instance responded with a permanent redirect (HTTP codes 301, 308)
    Moved { to: Host },
}

/// Messages that the checker can send to the orchestrator.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum CheckerResponse {
    /// The state of the instance.
    State { state: InstanceState },

    /// The instance peers with another instance, which is located at `hostname`.
    Peer { peer: Host },
}
