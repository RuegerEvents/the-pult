//! The wasmtime side: one engine for the process, and the bindings.
//!
//! The engine is shared because compilation caches and the epoch thread belong
//! to the process, not to a plugin. Epochs are the watchdog: a thread bumps the
//! counter every 10 ms, every guest call gets a deadline in ticks, and a guest
//! that blows through it traps out instead of taking the station with it.

use std::sync::OnceLock;
use std::time::Duration;

use wasmtime::component::Component;

/// How often the epoch advances. Deadlines are measured in these.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// How many ticks one guest call may take: five seconds. Generous, because an
/// LLM round trip happens inside a guest call; a plugin that computes for five
/// seconds straight is broken either way.
pub const CALL_DEADLINE_TICKS: u64 = 500;

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "plugin",
    imports: { default: async | trappable },
    exports: { default: async },
});

/// The process-wide engine. Started on first use; the ticker thread parks
/// itself for the life of the process, which costs nothing measurable.
pub fn engine() -> &'static wasmtime::Engine {
    static ENGINE: OnceLock<wasmtime::Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = wasmtime::Config::new();
        config.epoch_interruption(true);
        let engine = wasmtime::Engine::new(&config).expect("wasmtime engine configuration holds");
        let ticker = engine.clone();
        std::thread::Builder::new()
            .name("plugin-epoch".into())
            .spawn(move || loop {
                std::thread::sleep(EPOCH_TICK);
                ticker.increment_epoch();
            })
            .expect("spawning the epoch thread");
        engine
    })
}

/// Compile a component off the async runtime: cranelift takes real time, and a
/// hot reload should not stall every socket the station is serving.
pub async fn load_component(path: std::path::PathBuf) -> Result<Component, String> {
    let engine = engine().clone();
    match tokio::task::spawn_blocking(move || Component::from_file(&engine, &path)).await {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(join) => Err(join.to_string()),
    }
}
