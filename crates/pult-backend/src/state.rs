use pult_schema::events::operation::NodeId;

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
    /// Where this show's bytes are. A store rather than the pool it used to be: an
    /// asset is a file in the bundle now and a row in the database, and every caller
    /// needs both halves.
    pub assets: crate::infra::assets::AssetStore,
    pub sync: SyncHandle,
    pub session: SessionHandle,
    pub devices: DeviceHandle,
    pub plugins: PluginsHandle,
    /// One conversation with the GDTF Share for the whole station. Its session is a
    /// cookie, so two of these would be two logins where the Share expects one.
    pub share: crate::infra::interop::share::ShareHandle,
    pub node_id: NodeId,
    pub ws_registry: SubscriptionRegistry,
    pub broadcast: UpdateBroadcast,
    /// Which browsers are watching which peer's log. Consulted whenever a session
    /// comes or goes, so an ask never outlives the person making it.
    pub log_watchers: crate::logging::Watchers,
    /// Who is watching what an output is putting on the wire.
    pub viewers: crate::infra::connectors::Viewers,
    /// What the browsers this station is serving say they are costing themselves.
    /// LOCAL: a page belongs to the station holding its socket and to no other.
    pub clients: crate::infra::clients::ClientRegistry,
    pub config: Config,
    /// Where a show act goes, and what this station has open. The console does the
    /// opening; a station can only say it was asked.
    pub shows: crate::ShowsHandle,
    /// Flipped when this station is stopping, so every open WebSocket lets go.
    ///
    /// `axum::serve` gives each connection a task of its own, and those are not
    /// children of the future that accepted them: aborting the server leaves every
    /// socket talking to a station that has stopped. A browser in that state still
    /// says "Connected" and is subscribed to an engine that no longer exists, so it
    /// has to be told rather than left to notice.
    pub stopping: tokio::sync::watch::Receiver<bool>,
    /// The port that was actually bound, which is not `config.port` when that was
    /// zero. This is the one a client is talking to.
    pub http_port: u16,
}
