//! Blind, switched: the one act behind the top bar's button, the wing's BLIND key and
//! the `programmer.blind` RPC.
//!
//! **Unattributed**, the way a variable is. Switching blind is not an edit to the show
//! and must not be one Ctrl-Z away: an operator undoing a programmer change would
//! otherwise switch blind off on the way past and put the whole programmer on stage.
//! What being blind *does* is `Show::blind`'s doc and `engine::programmer_on_stage`.

use pult_schema::lifecycle::Lifecycle;
use pult_schema::path::PathSegment;

use crate::engine::EngineHandle;

fn path() -> Vec<PathSegment> {
    vec![PathSegment::Key("show".into()), PathSegment::Key("blind".into())]
}

/// Whether the show is blind now.
pub async fn is_blind(engine: &EngineHandle) -> bool {
    matches!(engine.get(path()).await, Ok(serde_json::Value::Bool(true)))
}

/// Switch blind on or off, or over with `None`, and answer what it is now.
pub async fn set(engine: &EngineHandle, on: Option<bool>) -> Result<bool, String> {
    let on = match on {
        Some(on) => on,
        None => !is_blind(engine).await,
    };
    engine.set(path(), Lifecycle::Synced, serde_json::Value::Bool(on)).await.map_err(|e| e.to_string())?;
    Ok(on)
}
