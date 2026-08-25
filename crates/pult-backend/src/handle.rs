use futures::stream::BoxStream;
use pult_schema::{
    handle::{DataHandle, HandleError},
    lifecycle::Lifecycle,
    path::{Path, PathPattern},
};

use crate::engine::EngineHandle;

/// Implements DataHandle by dispatching to the ShowEngine actor.
/// This is what backend business logic and WASM plugins use to
/// access the data model via the path-proxy API.
#[derive(Clone)]
#[allow(dead_code, reason = "the Rust accessor API entry point; used by the tests so far")]
pub struct EngineDataHandle(pub EngineHandle);

impl DataHandle for EngineDataHandle {
    async fn set(
        &self,
        path: Path,
        lifecycle: Lifecycle,
        value: serde_json::Value,
    ) -> Result<(), HandleError> {
        self.0
            .set(path, lifecycle, value)
            .await
            .map_err(|e| HandleError::Engine(e.to_string()))
    }

    async fn get(&self, path: Path) -> Result<serde_json::Value, HandleError> {
        self.0
            .get(path)
            .await
            .map_err(|e| HandleError::Engine(e.to_string()))
    }

    fn subscribe_raw(&self, path: Path) -> BoxStream<'static, serde_json::Value> {
        // Use pattern matching for the exact path
        let pattern = PathPattern::new(
            path.iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join("/"),
        );
        let handle = self.0.clone();
        Box::pin(async_stream::stream! {
            let mut stream = handle.subscribe_pattern(pattern).await;
            use futures::StreamExt;
            while let Some(v) = stream.next().await {
                yield v;
            }
        })
    }
}
