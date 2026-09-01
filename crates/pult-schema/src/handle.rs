use std::future::Future;
use std::marker::PhantomData;

use futures::{stream::BoxStream, Stream};
use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::{
    lifecycle::Lifecycle,
    path::{path_id, path_index, path_key, Path},
};

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum HandleError {
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("engine error: {0}")]
    Engine(String),
    #[error("path not found: {0:?}")]
    NotFound(Path),
}

// ── DataHandle trait ──────────────────────────────────────────────────────────

/// Central abstraction for path-based data access.
/// Implemented by EngineDataHandle (backend), WasmDataHandle (Phase 2),
/// and the TypeScript proxy on the frontend side.
pub trait DataHandle: Clone + Send + Sync + 'static {
    fn set(
        &self,
        path: Path,
        lifecycle: Lifecycle,
        value: serde_json::Value,
    ) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn get(
        &self,
        path: Path,
    ) -> impl Future<Output = Result<serde_json::Value, HandleError>> + Send;

    /// Returns a boxed `'static` stream so callers are not lifetime-bound to `&self`.
    fn subscribe_raw(&self, path: Path) -> BoxStream<'static, serde_json::Value>;
}

// ── FieldAccessor ─────────────────────────────────────────────────────────────

pub struct FieldAccessor<T, H: DataHandle> {
    path: Path,
    lifecycle: Lifecycle,
    handle: H,
    _t: PhantomData<T>,
}

impl<T, H: DataHandle> FieldAccessor<T, H> {
    pub fn new(path: Path, lifecycle: Lifecycle, handle: H) -> Self {
        Self { path, lifecycle, handle, _t: PhantomData }
    }
}

impl<T: Serialize + DeserializeOwned + Send + 'static, H: DataHandle> FieldAccessor<T, H> {
    pub async fn set(&self, value: T) -> Result<(), HandleError> {
        let json = serde_json::to_value(&value)?;
        self.handle.set(self.path.clone(), self.lifecycle, json).await
    }

    /// Move this field by `delta` rather than saying what it should become.
    ///
    /// Relative to what the field says at the moment the station applies it, not to
    /// what this caller last read — which is the point: two people nudging one value
    /// at once both get their nudge, where two people computing a destination from
    /// the same reading would leave only one.
    ///
    /// It is resolved to an absolute write before anything records or replicates it,
    /// so the history, the showfile and every peer see a destination. Only numeric
    /// fields and parameter values accept one.
    pub async fn by(&self, delta: f64) -> Result<(), HandleError> {
        let path = path_key(self.path.clone(), "__by");
        self.handle.set(path, self.lifecycle, serde_json::json!(delta)).await
    }

    pub async fn get(&self) -> Result<T, HandleError> {
        let json = self.handle.get(self.path.clone()).await?;
        serde_json::from_value(json).map_err(HandleError::Serialize)
    }

    pub fn subscribe(&self) -> impl Stream<Item = T> + 'static
    where
        T: DeserializeOwned,
    {
        use futures::StreamExt;
        let raw = self.handle.subscribe_raw(self.path.clone());
        raw.filter_map(|v| async move { serde_json::from_value(v).ok() })
    }
}

// ── LocalFieldAccessor ────────────────────────────────────────────────────────

/// Accessor for LOCAL lifecycle fields.
/// Sent to connected frontends but NOT replicated to peer backends.
pub struct LocalFieldAccessor<T, H: DataHandle> {
    path: Path,
    handle: H,
    _t: PhantomData<T>,
}

impl<T, H: DataHandle> LocalFieldAccessor<T, H> {
    pub fn new(path: Path, handle: H) -> Self {
        Self { path, handle, _t: PhantomData }
    }
}

impl<T: Serialize + DeserializeOwned + Send + 'static, H: DataHandle> LocalFieldAccessor<T, H> {
    pub async fn set(&self, value: T) -> Result<(), HandleError> {
        let json = serde_json::to_value(&value)?;
        self.handle.set(self.path.clone(), Lifecycle::Local, json).await
    }

    pub async fn get(&self) -> Result<T, HandleError> {
        let json = self.handle.get(self.path.clone()).await?;
        serde_json::from_value(json).map_err(HandleError::Serialize)
    }

    pub fn subscribe(&self) -> impl Stream<Item = T> + 'static
    where
        T: DeserializeOwned,
    {
        use futures::StreamExt;
        let raw = self.handle.subscribe_raw(self.path.clone());
        raw.filter_map(|v| async move { serde_json::from_value(v).ok() })
    }
}

// ── EntityAccessor trait ──────────────────────────────────────────────────────

pub trait EntityAccessor: Sized {
    type Value: Serialize + DeserializeOwned;
    type CreateValue: Serialize + DeserializeOwned;
    type Handle: DataHandle;

    fn new(path: Path, handle: Self::Handle) -> Self;
    fn path(&self) -> &Path;
    fn handle(&self) -> &Self::Handle;
}

// ── EntityCollectionAccessor ──────────────────────────────────────────────────

pub struct EntityCollectionAccessor<A: EntityAccessor> {
    path: Path,
    handle: A::Handle,
    _a: PhantomData<A>,
}

impl<A: EntityAccessor> EntityCollectionAccessor<A> {
    pub fn new(path: Path, handle: A::Handle) -> Self {
        Self { path, handle, _a: PhantomData }
    }

    /// Access by position in the ordered collection.
    pub fn nth(&self, index: usize) -> A {
        A::new(path_index(self.path.clone(), index), self.handle.clone())
    }

    /// Access by UUID.
    pub fn by_id(&self, id: Uuid) -> A {
        A::new(path_id(self.path.clone(), id), self.handle.clone())
    }

    pub async fn all(&self) -> Result<Vec<A::Value>, HandleError> {
        let json = self.handle.get(self.path.clone()).await?;
        serde_json::from_value(json).map_err(HandleError::Serialize)
    }

    pub fn subscribe_all(&self) -> impl Stream<Item = Vec<A::Value>> + 'static
    where
        A::Value: DeserializeOwned + 'static,
    {
        use futures::StreamExt;
        let raw = self.handle.subscribe_raw(self.path.clone());
        raw.filter_map(|v| async move { serde_json::from_value(v).ok() })
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), HandleError> {
        let delete_path = path_key(path_id(self.path.clone(), id), "__delete");
        self.handle.set(delete_path, Lifecycle::Persisted, serde_json::Value::Null).await
    }

    /// Put something back where it rests when nothing is driving it.
    ///
    /// The programmer's verb: `{ "fixtureId": <uuid> }` sends every output parameter
    /// of that fixture home, and naming a `parameterKind` as well sends just the one.
    /// Like [`FieldAccessor::by`], the station resolves it to ordinary absolute
    /// writes before anything records or replicates them, so a caller needs no way of
    /// reading the rig to ask for this — which is what lets a client that can set a
    /// level ask for home without being able to see one.
    ///
    /// A collection that has no such thing refuses it, naming the path.
    pub async fn home(&self, args: serde_json::Value) -> Result<(), HandleError> {
        let path = path_key(self.path.clone(), "__home");
        self.handle.set(path, Lifecycle::Synced, args).await
    }
}

// ── ShowDataRoot ──────────────────────────────────────────────────────────────

/// Top-level accessor for the whole show data tree.
/// Collection accessors are added by generated code (pult-schema types).
pub struct ShowDataRoot<H: DataHandle> {
    pub handle: H,
}

impl<H: DataHandle> ShowDataRoot<H> {
    pub fn new(handle: H) -> Self {
        Self { handle }
    }
}
