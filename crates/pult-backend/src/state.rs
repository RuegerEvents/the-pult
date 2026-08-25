use std::sync::Arc;

use pult_schema::events::operation::NodeId;
use sqlx::SqlitePool;

use crate::{
    engine::{EngineHandle, UpdateBroadcast},
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
    pub node_id: NodeId,
    pub ws_registry: SubscriptionRegistry,
    pub broadcast: UpdateBroadcast,
}
