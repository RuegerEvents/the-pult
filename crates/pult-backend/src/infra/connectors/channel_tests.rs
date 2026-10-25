//! Who is forcing channels onto which output, and to what.
//!
//! The same shape as [`super::Viewers`] and for the same reasons, which is why it
//! lives beside it and is reached through it: a browser asks through an RPC, a peer
//! asks down a sync link, and the connector cannot tell them apart. **Nothing
//! expires.** What is forced is recomputed from who is currently here — a tab that
//! closes is [`ChannelTests::forget`] and the same recompute, a station that vanishes
//! takes its connection and the test with it — so a light cannot be left at full by
//! a console that went away. That is the property that matters most, since this is the
//! one diagnostic that *drives* the rig.
//!
//! A session's test on an output is **wholesale**: what it sends replaces what it sent
//! before, and an empty test is the withdrawal. Several sessions on one output merge
//! with [`ChannelTest::merged`] — solo if any asks for it, the higher level on a
//! channel two of them name.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pult_schema::events::operation::NodeId;
use pult_schema::types::output::ChannelTest;
use tokio::sync::watch;
use uuid::Uuid;

/// One output, anywhere in the session.
type Forced = (NodeId, Uuid);

#[derive(Default)]
struct Inner {
    /// Per output, each session's whole test of it. A peer is one session carrying
    /// everything its own station's browsers asked for, folded into one entry.
    by_output: HashMap<Forced, HashMap<Uuid, ChannelTest>>,
}

#[derive(Clone)]
pub struct ChannelTests {
    inner: Arc<Mutex<Inner>>,
    /// Bumped whenever what an output is being forced to changes, so the output
    /// manager sleeps through the whole of a show in which nobody tests anything.
    changed: watch::Sender<u64>,
}

impl Default for ChannelTests {
    fn default() -> Self {
        let (changed, _) = watch::channel(0);
        ChannelTests { inner: Arc::new(Mutex::new(Inner::default())), changed }
    }
}

impl ChannelTests {
    /// Be told when any output's test changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Take one session's whole test of `output` on `node`, replacing what it sent
    /// before, and say what that output should now be forced to — `None` if the
    /// answer has not moved, and so there is nothing to tell anybody.
    ///
    /// The test is expected to have passed [`ChannelTest::checked`] already; this is
    /// a table and not the place a caller is refused.
    pub fn set(
        &self,
        node: NodeId,
        output: Uuid,
        session: Uuid,
        test: ChannelTest,
    ) -> Option<ChannelTest> {
        self.change(
            |inner| {
                let sessions = inner.by_output.entry((node, output)).or_default();
                if test.is_empty() {
                    sessions.remove(&session);
                } else {
                    sessions.insert(session, test);
                }
                if sessions.is_empty() {
                    inner.by_output.remove(&(node, output));
                }
            },
            (node, output),
        )
    }

    /// A browser is gone, or a peer's connection is. Says what every output it was
    /// forcing should now be forced to, which is the test that would otherwise
    /// outlive the person making it.
    pub fn forget(&self, session: Uuid) -> Vec<(Forced, ChannelTest)> {
        let forced: Vec<Forced> = {
            let inner = self.inner.lock().unwrap();
            inner
                .by_output
                .iter()
                .filter(|(_, sessions)| sessions.contains_key(&session))
                .map(|(key, _)| *key)
                .collect()
        };
        forced
            .into_iter()
            .filter_map(|(node, output)| {
                self.set(node, output, session, ChannelTest::default())
                    .map(|test| ((node, output), test))
            })
            .collect()
    }

    /// What an output is being forced to by everything this station knows of,
    /// whether or not the answer just moved. `None` where nothing is forcing it.
    pub fn merged_for(&self, node: NodeId, output: Uuid) -> Option<ChannelTest> {
        self.inner.lock().unwrap().by_output.get(&(node, output)).map(merged)
    }

    /// What this station's own connectors are being forced to, output by output.
    ///
    /// Only ours: an entry under another node is a test this station has put on a
    /// sync link, and acting on it here would be forcing a wire it does not hold.
    pub fn of(&self, node: NodeId) -> HashMap<Uuid, ChannelTest> {
        let inner = self.inner.lock().unwrap();
        inner
            .by_output
            .iter()
            .filter(|((owner, _), _)| *owner == node)
            .map(|((_, output), sessions)| (*output, merged(sessions)))
            .collect()
    }

    /// Apply a change and say whether the merged test of that output moved.
    fn change(&self, edit: impl FnOnce(&mut Inner), key: Forced) -> Option<ChannelTest> {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.by_output.get(&key).map(merged).unwrap_or_default();
        edit(&mut inner);
        let after = inner.by_output.get(&key).map(merged).unwrap_or_default();
        drop(inner);
        if before == after {
            return None;
        }
        self.changed.send_modify(|n| *n += 1);
        Some(after)
    }
}

fn merged(sessions: &HashMap<Uuid, ChannelTest>) -> ChannelTest {
    ChannelTest::merged(sessions.values().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pult_schema::types::output::TestUniverse;

    fn test(universe: u16, solo: bool, levels: &[(u16, u8)]) -> ChannelTest {
        ChannelTest {
            universes: vec![TestUniverse { universe, solo, levels: levels.iter().copied().collect() }],
        }
    }

    fn one() -> (ChannelTests, NodeId, Uuid) {
        (ChannelTests::default(), NodeId::new(), Uuid::new_v4())
    }

    #[test]
    fn a_test_and_its_withdrawal() {
        let (tests, node, output) = one();
        let alice = Uuid::new_v4();

        let forced = tests.set(node, output, alice, test(1, false, &[(5, 255)])).unwrap();
        assert_eq!(forced.universes[0].levels[&5], 255);
        assert_eq!(tests.of(node)[&output], forced);

        assert_eq!(tests.set(node, output, alice, ChannelTest::default()), Some(ChannelTest::default()));
        assert!(tests.of(node).is_empty(), "the last test leaving leaves nothing behind");
    }

    #[test]
    fn a_test_replaces_the_session_s_last_one_rather_than_adding_to_it() {
        let (tests, node, output) = one();
        let alice = Uuid::new_v4();
        tests.set(node, output, alice, test(1, false, &[(5, 255), (6, 255)]));
        let forced = tests.set(node, output, alice, test(1, false, &[(6, 255)])).unwrap();
        assert_eq!(forced.universes[0].levels.len(), 1, "channel 5 was let go, not left on");
    }

    #[test]
    fn a_test_that_has_not_moved_is_not_news() {
        let (tests, node, output) = one();
        let mut wake = tests.subscribe();
        let alice = Uuid::new_v4();

        tests.set(node, output, alice, test(1, false, &[(5, 255)]));
        assert!(wake.has_changed().unwrap());
        wake.mark_unchanged();

        assert_eq!(tests.set(node, output, alice, test(1, false, &[(5, 255)])), None);
        assert_eq!(
            tests.set(node, output, Uuid::new_v4(), test(1, false, &[(5, 100)])),
            None,
            "a second tester under the first on the same channel changes nothing"
        );
        assert!(!wake.has_changed().unwrap());
    }

    #[test]
    fn two_testers_merge_highest_takes_precedence_and_solo_if_either_asks() {
        let (tests, node, output) = one();
        tests.set(node, output, Uuid::new_v4(), test(1, false, &[(5, 100), (6, 10)]));
        let forced = tests.set(node, output, Uuid::new_v4(), test(1, true, &[(5, 200)])).unwrap();
        let universe = &forced.universes[0];
        assert!(universe.solo);
        assert_eq!(universe.levels[&5], 200);
        assert_eq!(universe.levels[&6], 10);
    }

    #[test]
    fn a_browser_that_vanishes_lets_go_of_everything_it_forced() {
        let (tests, node, other) = one();
        let alice = Uuid::new_v4();
        tests.set(node, other, alice, test(1, false, &[(1, 255)]));
        tests.set(node, Uuid::new_v4(), alice, test(2, true, &[]));
        let bob = Uuid::new_v4();
        tests.set(node, other, bob, test(1, false, &[(2, 255)]));

        let unwound = tests.forget(alice);
        assert_eq!(unwound.len(), 2);
        let left = &tests.of(node)[&other];
        assert_eq!(left.universes[0].levels.keys().copied().collect::<Vec<_>>(), vec![2], "bob's stays");
    }

    #[test]
    fn a_peers_output_is_forced_here_and_acted_on_nowhere() {
        let (tests, node, output) = one();
        let peer = NodeId::new();
        tests.set(peer, output, Uuid::new_v4(), test(1, false, &[(1, 255)]));
        assert!(tests.of(node).is_empty(), "this station holds no socket in that question");
        assert!(!tests.of(peer).is_empty());
    }
}
