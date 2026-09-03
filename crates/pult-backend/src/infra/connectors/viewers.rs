//! Who is looking at what a connector is putting on the wire.
//!
//! The same shape as [`crate::logging::Watchers`], and for the same reason: a view
//! of the traffic is expensive to produce and worth nothing to nobody, so it exists
//! while somebody is reading it and not otherwise. **Nothing expires.** An ask is a
//! function of who is currently watching, recomputed whenever that set changes and
//! sent on as the new answer — including the answer "nobody, stop". A browser that
//! closes its panel is one recompute; a browser that vanishes is [`Viewers::forget`]
//! and the same recompute; a station that vanishes takes its connection and the ask
//! with it.
//!
//! Two things are watched through here and they are deliberately the same table.
//! A *local* output is one this station runs, and the answer goes to its connector.
//! A *peer's* output is one another station runs, and the answer goes down the sync
//! link as [`crate::infra::sync::protocol::SyncMessage::OutputWatch`] — where the
//! receiving station registers it here again, under its own node id, with the peer
//! standing in for a session. So a peer watching and a browser watching are the same
//! entry in the same map, and a connector cannot tell them apart.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pult_schema::events::operation::NodeId;
use tokio::sync::watch;
use uuid::Uuid;

/// What one output is being asked for: the distinct parts of its traffic somebody
/// wants to see, sorted so that two stations arriving at the same ask spell it the
/// same way and nothing is re-sent for a reordering.
///
/// Empty means nobody is watching, which is the answer that stops a connector.
pub type Ask = Vec<Option<String>>;

/// One output, anywhere in the session.
type Watched = (NodeId, Uuid);

#[derive(Default)]
struct Inner {
    /// Per output, who is watching it and what each of them is looking at.
    ///
    /// A *set* per watcher rather than one focus, because a peer is one watcher
    /// carrying the whole of its own station's ask — every browser over there, folded
    /// into one entry here. A browser is the degenerate case with one focus in it.
    by_output: HashMap<Watched, HashMap<Uuid, Ask>>,
}

/// Who is watching which output's traffic, and at what part of it.
#[derive(Clone)]
pub struct Viewers {
    inner: Arc<Mutex<Inner>>,
    /// Bumped whenever anything changes, so the output manager can sleep instead of
    /// polling a map that is empty almost always. A station with no viewer open must
    /// cost nothing at all — which is the same promise the log makes and the reason
    /// there is no unconditional tick anywhere near the frame path.
    changed: watch::Sender<u64>,
}

impl Default for Viewers {
    fn default() -> Self {
        let (changed, _) = watch::channel(0);
        Viewers { inner: Arc::new(Mutex::new(Inner::default())), changed }
    }
}

impl Viewers {
    /// Be told when any ask changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Note that `session` is watching `output` on `node`, looking at `focus`, and
    /// say what that output should now be asked for — `None` if the answer has not
    /// changed, and so there is nothing to tell anybody.
    pub fn watch(
        &self,
        node: NodeId,
        output: Uuid,
        session: Uuid,
        focus: Option<String>,
    ) -> Option<Ask> {
        self.set(node, output, session, vec![focus])
    }

    /// Take one watcher's ask wholesale, replacing whatever it asked before.
    ///
    /// This is the peer's door: a station sends the whole of what it wants, so
    /// withdrawing half of it is the same message as asking for it, and there is no
    /// state on either side that can be left behind. An empty ask is the withdrawal.
    pub fn set(&self, node: NodeId, output: Uuid, session: Uuid, focuses: Ask) -> Option<Ask> {
        self.change(
            |inner| {
                let watchers = inner.by_output.entry((node, output)).or_default();
                if focuses.is_empty() {
                    watchers.remove(&session);
                } else {
                    watchers.insert(session, focuses);
                }
                if watchers.is_empty() {
                    inner.by_output.remove(&(node, output));
                }
            },
            (node, output),
        )
    }

    /// Note that `session` has stopped watching.
    pub fn unwatch(&self, node: NodeId, output: Uuid, session: Uuid) -> Option<Ask> {
        self.change(|inner| {
            if let Some(watchers) = inner.by_output.get_mut(&(node, output)) {
                watchers.remove(&session);
                if watchers.is_empty() {
                    inner.by_output.remove(&(node, output));
                }
            }
        }, (node, output))
    }

    /// A browser is gone, or a peer's connection is. Says what every output it was
    /// watching should now be asked for, which is the ask that would otherwise
    /// outlive the person making it.
    pub fn forget(&self, session: Uuid) -> Vec<(Watched, Ask)> {
        let watched: Vec<Watched> = {
            let inner = self.inner.lock().unwrap();
            inner
                .by_output
                .iter()
                .filter(|(_, watchers)| watchers.contains_key(&session))
                .map(|(key, _)| *key)
                .collect()
        };
        watched
            .into_iter()
            .filter_map(|(node, output)| {
                self.unwatch(node, output, session).map(|ask| ((node, output), ask))
            })
            .collect()
    }

    /// What this station's own connectors are being asked for, output by output.
    ///
    /// Only ours: an entry under another node is an ask this station has put on a
    /// sync link, and drawing it here would be answering a question about somebody
    /// else's wire.
    pub fn asks_of(&self, node: NodeId) -> Vec<(Uuid, Ask)> {
        let inner = self.inner.lock().unwrap();
        let mut asks: Vec<(Uuid, Ask)> = inner
            .by_output
            .iter()
            .filter(|((owner, _), _)| *owner == node)
            .map(|((_, output), watchers)| (*output, distinct(watchers)))
            .collect();
        asks.sort_by_key(|(output, _)| *output);
        asks
    }

    /// Is anybody watching anything of this station's?
    pub fn any_on(&self, node: NodeId) -> bool {
        self.inner.lock().unwrap().by_output.keys().any(|(owner, _)| *owner == node)
    }

    /// Apply a change and say whether the ask for that output moved.
    fn change(&self, edit: impl FnOnce(&mut Inner), key: Watched) -> Option<Ask> {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.by_output.get(&key).map(distinct).unwrap_or_default();
        edit(&mut inner);
        let after = inner.by_output.get(&key).map(distinct).unwrap_or_default();
        drop(inner);
        if before == after {
            return None;
        }
        self.changed.send_modify(|n| *n += 1);
        Some(after)
    }
}

/// The distinct things being asked for, in a stable order.
///
/// Two browsers on two universes of one output are two asks, not one won by
/// whoever spoke last — the connector is asked once per focus and answers each.
fn distinct(watchers: &HashMap<Uuid, Ask>) -> Ask {
    let mut focuses: Vec<Option<String>> = watchers.values().flatten().cloned().collect();
    focuses.sort();
    focuses.dedup();
    focuses
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one() -> (Viewers, NodeId, Uuid) {
        (Viewers::default(), NodeId::new(), Uuid::new_v4())
    }

    #[test]
    fn watching_and_letting_go_are_the_ask_and_its_withdrawal() {
        let (viewers, node, output) = one();
        let alice = Uuid::new_v4();

        assert_eq!(
            viewers.watch(node, output, alice, Some("1".into())),
            Some(vec![Some("1".to_string())])
        );
        assert!(viewers.any_on(node));
        assert_eq!(viewers.unwatch(node, output, alice), Some(vec![]), "nobody, stop");
        assert!(!viewers.any_on(node), "the last viewer leaving leaves nothing behind");
    }

    #[test]
    fn an_ask_that_has_not_moved_is_not_re_sent() {
        let (viewers, node, output) = one();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        viewers.watch(node, output, alice, Some("1".into()));
        assert_eq!(
            viewers.watch(node, output, bob, Some("1".into())),
            None,
            "a second browser on the same universe changes nothing on the wire"
        );
        assert_eq!(
            viewers.unwatch(node, output, alice),
            None,
            "and one of them leaving changes nothing either, because the other is still here"
        );
        assert_eq!(viewers.unwatch(node, output, bob), Some(vec![]));
    }

    #[test]
    fn two_viewers_on_two_universes_are_two_asks() {
        let (viewers, node, output) = one();
        viewers.watch(node, output, Uuid::new_v4(), Some("1".into()));
        let ask = viewers.watch(node, output, Uuid::new_v4(), Some("5".into())).unwrap();
        assert_eq!(ask, vec![Some("1".to_string()), Some("5".to_string())]);
        assert_eq!(viewers.asks_of(node), vec![(output, ask)]);
    }

    #[test]
    fn a_browser_that_vanishes_takes_every_ask_it_made() {
        let (viewers, node, output) = one();
        let other = Uuid::new_v4();
        let alice = Uuid::new_v4();
        viewers.watch(node, output, alice, None);
        viewers.watch(node, other, alice, Some("7".into()));

        let unwound = viewers.forget(alice);
        assert_eq!(unwound.len(), 2, "both of them, and both as 'nobody'");
        assert!(unwound.iter().all(|(_, ask)| ask.is_empty()));
        assert!(!viewers.any_on(node));
    }

    #[test]
    fn a_peer_asks_wholesale_and_withdraws_the_same_way() {
        let (viewers, node, output) = one();
        let peer = Uuid::new_v4();

        let ask = viewers
            .set(node, output, peer, vec![Some("1".into()), Some("5".into())])
            .unwrap();
        assert_eq!(ask, vec![Some("1".to_string()), Some("5".to_string())]);
        assert_eq!(
            viewers.set(node, output, peer, vec![Some("1".into())]),
            Some(vec![Some("1".to_string())]),
            "dropping one of two is one message, not a withdrawal and a re-ask"
        );
        assert_eq!(viewers.set(node, output, peer, vec![]), Some(vec![]));
        assert!(!viewers.any_on(node), "an empty ask is the withdrawal");
    }

    #[test]
    fn a_peers_output_is_watched_here_and_drawn_nowhere() {
        let (viewers, node, output) = one();
        let peer = NodeId::new();
        viewers.watch(peer, output, Uuid::new_v4(), Some("3".into()));

        assert!(viewers.any_on(peer));
        assert!(!viewers.any_on(node), "this station has no wire in that question");
        assert!(viewers.asks_of(node).is_empty());
    }

    #[test]
    fn the_manager_is_woken_when_an_ask_moves_and_not_otherwise() {
        let (viewers, node, output) = one();
        let mut wake = viewers.subscribe();
        let alice = Uuid::new_v4();

        viewers.watch(node, output, alice, None);
        assert!(wake.has_changed().unwrap());
        wake.mark_unchanged();

        viewers.watch(node, output, Uuid::new_v4(), None);
        assert!(!wake.has_changed().unwrap(), "the same ask twice is not news");
    }
}
