use std::sync::Arc;

use pult_schema::events::operation::NodeId;
use sqlx::SqlitePool;

use crate::{
    config::Config,
    engine::{EngineHandle, UpdateBroadcast},
    infra::devices::DeviceHandle,
    infra::plugins::PluginsHandle,
    infra::session::SessionHandle,
    infra::sync::SyncHandle,
    api::ws::SubscriptionRegistry,
};

#[derive(Clone)]
#[allow(dead_code, reason = "shared state held for handlers that do not read it yet")]
pub struct AppState {
    pub engine: EngineHandle,
    pub pool: Arc<SqlitePool>,
    pub sync: SyncHandle,
    pub session: SessionHandle,
    pub devices: DeviceHandle,
    pub plugins: PluginsHandle,
    pub node_id: NodeId,
    pub ws_registry: SubscriptionRegistry,
    pub broadcast: UpdateBroadcast,
    pub config: Config,
    /// The port that was actually bound, which is not `config.port` when that was
    /// zero. This is the one a client is talking to.
    pub http_port: u16,
}
