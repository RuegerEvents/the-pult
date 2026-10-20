//! Which broker a node is on, and which it tries next — doc 15, the way the firmware's
//! `oh_brokers` does it.
//!
//! Pure, so the order a node walks its list in is a test rather than something read
//! off a log at the venue. The firmware's copy is C and has its own host tests; the
//! two are kept to one rule by being short enough to read side by side.

use std::time::Duration;

/// How long a node waits after a whole pass of its list has failed, doubling from
/// [`FIRST_BACKOFF`] up to [`MAX_BACKOFF`]. Between entries of one pass it waits
/// [`BETWEEN_ENTRIES`], so a list of dead addresses is not a tight loop.
pub const FIRST_BACKOFF: Duration = Duration::from_secs(1);
pub const MAX_BACKOFF: Duration = Duration::from_secs(8);
pub const BETWEEN_ENTRIES: Duration = Duration::from_millis(200);

/// How many entries a node keeps. More than this is a session with a spare for every
/// node anyway.
pub const MAX_ENTRIES: usize = 4;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrokerList {
    entries: Vec<String>,
    /// The entry being tried, or connected to.
    current: usize,
    /// What the next full-pass failure waits.
    backoff: Option<Duration>,
}

impl BrokerList {
    /// The entries, leader first.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// The broker to try now, or none for a node nobody has told anything.
    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.current).map(String::as_str)
    }

    /// Take a new list. Answers whether the node has to **move**: whenever the first
    /// entry is not the broker it is connected to. An empty list is ignored — a node
    /// that forgot every broker could not be reached to be told another.
    ///
    /// Moving starts again from the top, because the first entry is the leader and
    /// the leader is the only station whose door is open.
    pub fn set(&mut self, new: &[String], connected_to: Option<&str>) -> bool {
        let new: Vec<String> = new.iter().take(MAX_ENTRIES).cloned().collect();
        if new.is_empty() {
            return false;
        }
        let unchanged = new == self.entries;
        self.entries = new;
        if connected_to == Some(self.entries[0].as_str()) {
            self.current = 0;
            return false;
        }
        if unchanged && connected_to.is_none() {
            // Still walking the same list: carry on where it was.
            return false;
        }
        self.current = 0;
        self.backoff = None;
        true
    }

    /// The broker being tried refused or dropped. Moves to the next entry and says how
    /// long to wait before trying it.
    pub fn failed(&mut self) -> Duration {
        if self.entries.is_empty() {
            return MAX_BACKOFF;
        }
        self.current = (self.current + 1) % self.entries.len();
        if self.current != 0 {
            return BETWEEN_ENTRIES;
        }
        // A whole pass failed.
        let wait = self.backoff.unwrap_or(FIRST_BACKOFF);
        self.backoff = Some((wait * 2).min(MAX_BACKOFF));
        wait
    }

    /// Connected: the next failure starts the backoff from the beginning.
    pub fn connected(&mut self) {
        self.backoff = None;
    }
}

/// The list a `POST /api/v1/config` body or an `openhaunt/brokers` message carries:
/// `brokers` when there is one, `broker` alone as a one-entry list otherwise.
pub fn from_json(mqtt: &serde_json::Value) -> Vec<String> {
    if let Some(list) = mqtt["brokers"].as_array() {
        return list.iter().filter_map(|b| b.as_str().map(String::from)).filter(|b| !b.is_empty()).collect();
    }
    mqtt["broker"].as_str().filter(|b| !b.is_empty()).map(|b| vec![b.to_string()]).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|e| e.to_string()).collect()
    }

    #[test]
    fn a_node_walks_its_list_in_order_and_backs_off_after_a_whole_pass() {
        let mut brokers = BrokerList::default();
        assert!(brokers.set(&list(&["leader:1883", "spare:1883"]), None));
        assert_eq!(brokers.current(), Some("leader:1883"));

        assert_eq!(brokers.failed(), BETWEEN_ENTRIES);
        assert_eq!(brokers.current(), Some("spare:1883"));
        assert_eq!(brokers.failed(), FIRST_BACKOFF, "the whole list failed once");
        assert_eq!(brokers.current(), Some("leader:1883"), "and it starts at the top again");
        brokers.failed();
        assert_eq!(brokers.failed(), FIRST_BACKOFF * 2, "doubling");
        for _ in 0..10 {
            brokers.failed();
        }
        assert_eq!(brokers.failed(), BETWEEN_ENTRIES);
        assert_eq!(brokers.failed(), MAX_BACKOFF, "and no further than the cap");

        brokers.connected();
        brokers.failed();
        assert_eq!(brokers.failed(), FIRST_BACKOFF, "a connection resets the backoff");
    }

    #[test]
    fn a_new_leader_first_is_a_move_and_the_same_one_is_not() {
        let mut brokers = BrokerList::default();
        brokers.set(&list(&["a:1", "b:1"]), None);
        assert!(!brokers.set(&list(&["a:1", "b:1", "c:1"]), Some("a:1")), "a longer tail moves nobody");
        assert!(brokers.set(&list(&["b:1", "a:1"]), Some("a:1")), "a different leader does");
        assert_eq!(brokers.current(), Some("b:1"));
        // Connected to the spare after walking there, and the spare now leads.
        brokers.failed();
        assert!(!brokers.set(&list(&["a:1", "b:1"]), Some("a:1")), "already where the list says");
        assert_eq!(brokers.current(), Some("a:1"));
    }

    #[test]
    fn an_empty_list_is_not_a_reason_to_forget_the_one_it_has() {
        let mut brokers = BrokerList::default();
        brokers.set(&list(&["a:1"]), None);
        assert!(!brokers.set(&[], Some("a:1")));
        assert_eq!(brokers.entries(), list(&["a:1"]).as_slice());
    }

    #[test]
    fn a_list_is_capped_where_the_firmware_caps_it() {
        let mut brokers = BrokerList::default();
        brokers.set(&list(&["a:1", "b:1", "c:1", "d:1", "e:1"]), None);
        assert_eq!(brokers.entries().len(), MAX_ENTRIES);
    }

    #[test]
    fn a_console_from_before_lists_is_a_list_of_one() {
        assert_eq!(from_json(&serde_json::json!({ "broker": "a:1" })), list(&["a:1"]));
        assert_eq!(
            from_json(&serde_json::json!({ "broker": "a:1", "brokers": ["a:1", "b:1"] })),
            list(&["a:1", "b:1"]),
            "the list wins where there is one",
        );
        assert!(from_json(&serde_json::json!({ "broker": "" })).is_empty());
    }
}
