//! A plugin that exists to be tested against.
//!
//! The store interface is a *host* interface: the guest side is four function
//! calls, and everything worth asserting — where the data goes, whether it
//! replicates, whether the write is the operator's, what the quota does —
//! happens on the other side of the boundary. So the only honest way to test it
//! is from a guest that calls it on demand, which is all this is.
//!
//! Every method forwards to the store interface and reports what came back. The
//! one method that does more is `set-repeatedly`, which writes the same key many
//! times inside a *single* inbound call: one inbound call is one gesture, so
//! that is what within-gesture coalescing has to collapse, and it cannot be
//! provoked from outside.

use pult_plugin_sdk::{self as sdk, PultPlugin};
use serde_json::{json, Value};

struct StoreProbe;

/// `args.store` and `args.key`, which every method here takes.
fn store_and_key(args: &Value) -> (String, String) {
    (
        args["store"].as_str().unwrap_or_default().to_string(),
        args["key"].as_str().unwrap_or_default().to_string(),
    )
}

impl PultPlugin for StoreProbe {
    fn init(_config: Value) -> Result<Self, String> {
        Ok(StoreProbe)
    }

    fn handle(&mut self, method: &str, args: Value, _ctx: Value) -> Result<Value, String> {
        match method {
            "get" => {
                let (store, key) = store_and_key(&args);
                sdk::store::get_value(&store, &key)
            }
            "set" => {
                let (store, key) = store_and_key(&args);
                sdk::store::set(&store, &key, &args["value"])?;
                Ok(json!("ok"))
            }
            "delete" => {
                let (store, key) = store_and_key(&args);
                sdk::store::delete(&store, &key)?;
                Ok(json!("ok"))
            }
            "keys" => {
                let store = args["store"].as_str().unwrap_or_default();
                let prefix = args["prefix"].as_str().unwrap_or("");
                sdk::store::keys(store, prefix).map(Value::from)
            }
            // Write one key `times` times, inside this one call. What the log
            // does with that is the property being tested.
            "set-repeatedly" => {
                let (store, key) = store_and_key(&args);
                let times = args["times"].as_u64().unwrap_or(1);
                for n in 0..times {
                    sdk::store::set(&store, &key, &json!({ "n": n }))?;
                }
                Ok(json!({ "wrote": times }))
            }
            other => Err(format!("no method called {other:?}")),
        }
    }
}

sdk::plugin_main!(StoreProbe);
