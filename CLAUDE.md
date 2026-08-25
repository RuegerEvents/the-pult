# the-pult

Distributed lighting console system.

## Architecture

- **`crates/pult-macros`** — `#[derive(PultSchema)]` proc macro. Generates `PultEntity` impl, `{T}Patch`, `{T}Create`, `{T}Accessor` from annotated Rust structs.
- **`crates/pult-schema`** — Data model + path accessor infrastructure. All entity types live here. Source of truth for the WebSocket protocol and sync protocol.
- **`crates/pult-backend`** — Main backend binary. Axum WebSocket server, SQLite showfiles, peer sync (mDNS + TCP), WASM plugin runtime (Phase 2), fixture connectors (Phase 2).
- **`tools/pult-codegen`** — CLI that triggers ts-rs TypeScript export and writes `frontend/src/lib/generated/`.
- **`frontend/`** — SvelteKit static-adapter frontend.

## Lifecycle System

Every field in the data model has one of three lifecycles:
- `LOCAL` — stays on this backend node; synced to connected frontends but NOT to peer backends, not persisted.
- `SYNCED` — broadcast to all peer backends AND all connected frontends; not persisted.
- `PERSISTED` — written to SQLite AND replicated to peers AND frontends.

Frontend-only UI state (selections, hover, expanded rows) lives in Svelte stores — not in the schema.

## Path-Based Access API

Everything is accessed via a path-proxy:
- Rust backend: `data.sequences().nth(5).cues().nth(3).fade_time().set(4.0).await?`
- TypeScript frontend: `await data.sequences[5].cues[3].fadeTime.set(4)`

## Design Principle: pult-schema is the single source of truth

All entity types live in `pult-schema`. When the data model changes, **no other location should need a manual update**. Specifically:

- Do not enumerate entity types or collection names in the sync protocol, snapshot structures, or codec logic. Use serde-derived serialization of `ShowState` as a whole.
- The one allowed maintenance point when adding a new entity collection is `ShowState` in `engine/mod.rs`: add the field and add it to `ShowState::FRONTEND_PATHS`. Nowhere else.

## After Changing Schema Types

Run the TypeScript codegen after any change to types in `pult-schema`:
```
cargo run -p pult-codegen -- generate
```

## Running

```
cargo run -p pult-backend
cd frontend && npm run dev
```

## Testing

```
cargo test --workspace
```
