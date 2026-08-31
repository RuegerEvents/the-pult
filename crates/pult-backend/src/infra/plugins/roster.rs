//! What the show asks for, against what this station is running.
//!
//! The diff is a pure function of two lists so it can be tested without a
//! station, a showfile or a wasm engine. Everything it decides is keyed by
//! `(plugin_id, sha256)`: a digest names bytes, and bytes are what a running
//! plugin actually is.
//!
//! The case worth naming is [`Action::Publish`]. A roster row carries a display
//! name and a stage hint as well as a digest, and neither changes what runs — so
//! editing them must not restart the plugin. Task 9 learned this the expensive
//! way with outputs: rebuilding a live thing because its label changed put a
//! redundant frame on the wire for a typo. Here it would be a plugin restarting
//! during a show.

use std::collections::BTreeMap;

use pult_schema::types::plugin::PluginPackage;

/// What this station is running for one plugin id, as far as the diff cares.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    /// The digest it was started from, or `None` for a plugin loaded off disk.
    pub sha256: Option<String>,
}

/// One thing to do about one plugin id.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Not running, and wanted: unpack (or fetch) this digest and start it.
    Start { plugin_id: String, sha256: String },
    /// Running the wrong digest: stop what is there and start this instead.
    Replace { plugin_id: String, sha256: String },
    /// Running and no longer wanted, or disabled.
    Stop { plugin_id: String },
    /// Nothing about what runs changed — only what is displayed. Refresh the
    /// published state and leave the instance alone.
    Publish { plugin_id: String },
}

/// What to do to make `running` match `roster`.
///
/// `overridden` are plugin ids this station loads from a directory. They are
/// removed from consideration entirely: the disk copy wins here, and the roster
/// has no say about a plugin somebody is editing.
pub fn plan(
    roster: &[PluginPackage],
    running: &BTreeMap<String, Running>,
    overridden: &[String],
) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut wanted: BTreeMap<&str, &PluginPackage> = BTreeMap::new();

    for package in roster {
        if overridden.iter().any(|id| id == &package.plugin_id) {
            continue;
        }
        // A duplicate plugin id should not exist — the install path enforces
        // that — but the roster is replicated data and two stations can install
        // at once. Taking the last one is arbitrary and stable, which is what
        // matters: every station reading the same rows makes the same choice.
        wanted.insert(package.plugin_id.as_str(), package);
    }

    for (plugin_id, package) in &wanted {
        let current = running.get(*plugin_id);
        match (package.enabled, current) {
            // Disabled and running: stop, but keep the row and its config.
            (false, Some(_)) => actions.push(Action::Stop { plugin_id: plugin_id.to_string() }),
            (false, None) => {}
            (true, None) => actions.push(Action::Start {
                plugin_id: plugin_id.to_string(),
                sha256: package.sha256.clone(),
            }),
            (true, Some(Running { sha256: Some(have) })) if have == &package.sha256 => {
                actions.push(Action::Publish { plugin_id: plugin_id.to_string() })
            }
            (true, Some(_)) => actions.push(Action::Replace {
                plugin_id: plugin_id.to_string(),
                sha256: package.sha256.clone(),
            }),
        }
    }

    // Running from a digest, and the row that asked for it is gone.
    for (plugin_id, state) in running {
        if state.sha256.is_some()
            && !wanted.contains_key(plugin_id.as_str())
            && !overridden.iter().any(|id| id == plugin_id)
        {
            actions.push(Action::Stop { plugin_id: plugin_id.clone() });
        }
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use pult_schema::types::plugin::PluginStage;
    use uuid::Uuid;

    fn package(plugin_id: &str, sha: &str) -> PluginPackage {
        PluginPackage {
            id: Uuid::new_v4(),
            plugin_id: plugin_id.into(),
            name: plugin_id.into(),
            version: "0.1.0".into(),
            api: "0.1".into(),
            sha256: sha.into(),
            enabled: true,
            stage: PluginStage::Both,
            config: serde_json::Value::Null,
        }
    }

    fn running(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Running> {
        pairs
            .iter()
            .map(|(id, sha)| (id.to_string(), Running { sha256: sha.map(|s| s.to_string()) }))
            .collect()
    }

    #[test]
    fn a_package_nothing_is_running_is_started() {
        let actions = plan(&[package("command-line", "aaa")], &running(&[]), &[]);
        assert_eq!(actions, vec![Action::Start { plugin_id: "command-line".into(), sha256: "aaa".into() }]);
    }

    #[test]
    fn a_package_already_running_at_that_digest_is_only_published() {
        // The case that matters: a rename must not restart a plugin mid-show.
        // Nothing about the digest changed, so nothing about what runs changed.
        let mut renamed = package("command-line", "aaa");
        renamed.name = "The Command Line".into();
        renamed.stage = PluginStage::Setup;

        let actions = plan(&[renamed], &running(&[("command-line", Some("aaa"))]), &[]);

        assert_eq!(actions, vec![Action::Publish { plugin_id: "command-line".into() }]);
    }

    #[test]
    fn a_changed_digest_replaces_what_is_running() {
        let actions = plan(
            &[package("command-line", "bbb")],
            &running(&[("command-line", Some("aaa"))]),
            &[],
        );
        assert_eq!(
            actions,
            vec![Action::Replace { plugin_id: "command-line".into(), sha256: "bbb".into() }]
        );
    }

    #[test]
    fn a_package_removed_from_the_roster_is_stopped() {
        let actions = plan(&[], &running(&[("command-line", Some("aaa"))]), &[]);
        assert_eq!(actions, vec![Action::Stop { plugin_id: "command-line".into() }]);
    }

    #[test]
    fn a_disabled_package_is_stopped_but_stays_a_row() {
        let mut off = package("command-line", "aaa");
        off.enabled = false;

        let actions = plan(&[off.clone()], &running(&[("command-line", Some("aaa"))]), &[]);
        assert_eq!(actions, vec![Action::Stop { plugin_id: "command-line".into() }]);

        // And re-enabling starts it again from the same digest.
        let actions = plan(&[package("command-line", "aaa")], &running(&[]), &[]);
        assert_eq!(actions, vec![Action::Start { plugin_id: "command-line".into(), sha256: "aaa".into() }]);
    }

    #[test]
    fn a_plugin_loaded_from_disk_is_not_the_rosters_business() {
        let overridden = vec!["command-line".to_string()];

        // The roster wants a different digest; the disk copy stays put and is
        // not replaced, stopped, or counted as missing.
        let actions = plan(
            &[package("command-line", "bbb")],
            &running(&[("command-line", None)]),
            &overridden,
        );
        assert!(actions.is_empty(), "{actions:?}");

        // And removing the row does not stop the disk copy either.
        let actions = plan(&[], &running(&[("command-line", None)]), &overridden);
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn a_directory_plugin_the_roster_never_mentioned_is_left_alone() {
        // Its `sha256` is None, so it is not something the roster stopped
        // asking for — it was never the roster's.
        let actions = plan(&[], &running(&[("scratch", None)]), &["scratch".to_string()]);
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn several_packages_are_each_decided_on_their_own() {
        let actions = plan(
            &[package("a", "1"), package("b", "2"), package("c", "3")],
            &running(&[("a", Some("1")), ("b", Some("old")), ("d", Some("4"))]),
            &[],
        );

        assert!(actions.contains(&Action::Publish { plugin_id: "a".into() }));
        assert!(actions.contains(&Action::Replace { plugin_id: "b".into(), sha256: "2".into() }));
        assert!(actions.contains(&Action::Start { plugin_id: "c".into(), sha256: "3".into() }));
        assert!(actions.contains(&Action::Stop { plugin_id: "d".into() }));
        assert_eq!(actions.len(), 4);
    }
}
