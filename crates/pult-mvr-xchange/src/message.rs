//! The messages, as JSON.
//!
//! Field names are the specification's own — `FileUUID`, `verMajor`, `OK` — spelled
//! out one at a time rather than derived by a rename rule, because there is no rule
//! that produces all three.
//!
//! # Where this is lenient, and why
//!
//! The published document disagrees with its own worked examples in three places.
//! Every one of them is somewhere a strict parser would refuse a message that real
//! software sends, so each is read both ways and written the way the *table* says:
//!
//! 1. `MVR_LEAVE` carries `FromStationUUID` in the table and `StationUUID` in the
//!    example. Both are accepted.
//! 2. `MVR_REQUEST`'s `FromStationUUID` is typed as an array of uuid in the table and
//!    written as a single string in the example. Both are accepted; one is sent.
//! 3. `MVR_COMMIT`'s example omits `StationUUID` and `FileName`, which the table has.
//!    Missing fields are defaulted rather than fatal.
//!
//! And a message type this build has never heard of parses as [`Message::Unknown`]
//! rather than killing the connection it arrived on. A group can carry a message
//! meant for somebody else; going deaf over it would be worse than ignoring it.

use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

/// One revision of a project, as announced. The file itself is fetched separately —
/// a commit is a claim that bytes exist somewhere, not the bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Commit {
    /// The MVR file format's version, not the protocol's.
    #[serde(rename = "verMajor", default)]
    pub ver_major: u32,
    #[serde(rename = "verMinor", default)]
    pub ver_minor: u32,
    #[serde(rename = "FileSize", default)]
    pub file_size: u64,
    #[serde(rename = "FileUUID")]
    pub file_uuid: Uuid,
    /// Who made it. Defaulted because the specification's own example leaves it out,
    /// and a commit from nobody is still a commit somebody can ask for.
    #[serde(rename = "StationUUID", default)]
    pub station_uuid: Uuid,
    /// Who it is for. Empty is everybody, which is the only kind this console sends.
    #[serde(rename = "ForStationsUUID", default, skip_serializing_if = "Vec::is_empty")]
    pub for_stations: Vec<Uuid>,
    #[serde(rename = "Comment", default)]
    pub comment: String,
    #[serde(rename = "FileName", default)]
    pub file_name: String,
}

impl Commit {
    /// Is this commit addressed to `station`?
    ///
    /// An empty `ForStationsUUID` is the broadcast, which is what almost every commit
    /// is. A commit naming other stations is not ours to list, let alone apply — the
    /// sender took the trouble to say who it was for.
    pub fn is_for(&self, station: Uuid) -> bool {
        self.for_stations.is_empty() || self.for_stations.contains(&station)
    }
}

/// What every `*_RET` message carries, and nothing else does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    #[serde(rename = "OK")]
    pub ok: bool,
    #[serde(rename = "Message", default)]
    pub message: String,
}

impl Response {
    pub fn ok() -> Self {
        Response { ok: true, message: String::new() }
    }

    pub fn failed(why: impl Into<String>) -> Self {
        Response { ok: false, message: why.into() }
    }
}

/// Every message the protocol defines, in both modes.
///
/// Internally tagged on `Type`, which is how the wire spells it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "Type")]
pub enum Message {
    /// "I am here, and this is what I have." Sent on connecting, and again whenever
    /// this station's name or commit list changes.
    #[serde(rename = "MVR_JOIN")]
    Join {
        #[serde(rename = "Provider", default)]
        provider: String,
        #[serde(rename = "verMajor", default)]
        ver_major: u32,
        #[serde(rename = "verMinor", default)]
        ver_minor: u32,
        #[serde(rename = "StationUUID")]
        station_uuid: Uuid,
        #[serde(rename = "StationName", default)]
        station_name: String,
        #[serde(rename = "Commits", default)]
        commits: Vec<Commit>,
    },
    #[serde(rename = "MVR_JOIN_RET")]
    JoinRet {
        #[serde(flatten)]
        response: Response,
        #[serde(rename = "Provider", default)]
        provider: String,
        #[serde(rename = "verMajor", default)]
        ver_major: u32,
        #[serde(rename = "verMinor", default)]
        ver_minor: u32,
        #[serde(rename = "StationUUID", default)]
        station_uuid: Uuid,
        #[serde(rename = "StationName", default)]
        station_name: String,
        #[serde(rename = "Commits", default)]
        commits: Vec<Commit>,
    },
    /// "Stop telling me about new files." The mDNS service may stay up; leaving a
    /// group and leaving the network are different acts.
    #[serde(rename = "MVR_LEAVE")]
    Leave {
        #[serde(rename = "FromStationUUID", alias = "StationUUID", default)]
        from_station: Uuid,
    },
    #[serde(rename = "MVR_LEAVE_RET")]
    LeaveRet {
        #[serde(flatten)]
        response: Response,
    },
    /// "There is a new revision." Announcement only; the bytes move on request.
    #[serde(rename = "MVR_COMMIT")]
    Commit(Commit),
    #[serde(rename = "MVR_COMMIT_RET")]
    CommitRet {
        #[serde(flatten)]
        response: Response,
    },
    /// "Send me that file." No `FileUUID` asks for the sender's latest.
    #[serde(rename = "MVR_REQUEST")]
    Request {
        #[serde(rename = "FileUUID", default, skip_serializing_if = "Option::is_none")]
        file_uuid: Option<Uuid>,
        /// Who is being asked. Read as either one uuid or an array of them, because
        /// the specification says both in two places; written as one.
        #[serde(
            rename = "FromStationUUID",
            default,
            deserialize_with = "one_or_many",
            skip_serializing_if = "Option::is_none"
        )]
        from_station: Option<Uuid>,
    },
    /// The *failure* answer to a request. Success is a file, and a file is not JSON —
    /// in TCP mode it is a package of kind [`crate::frame::Payload::File`], in
    /// WebSocket mode a binary frame.
    #[serde(rename = "MVR_REQUEST_RET")]
    RequestRet {
        #[serde(flatten)]
        response: Response,
    },
    /// "This group is moving." Either to a different mDNS service name, or to a
    /// WebSocket URL — never both, and setting both is an error the receiver reports.
    #[serde(rename = "MVR_NEW_SESSION_HOST")]
    NewSessionHost {
        #[serde(rename = "ServiceName", default)]
        service_name: String,
        #[serde(rename = "ServiceURL", default)]
        service_url: String,
    },
    #[serde(rename = "MVR_NEW_SESSION_HOST_RET")]
    NewSessionHostRet {
        #[serde(flatten)]
        response: Response,
    },
    /// A type this build does not know.
    ///
    /// Kept rather than refused: the protocol is versioned by nothing, groups carry
    /// other people's software, and a console that drops a connection over a message
    /// it did not recognise is a console that stops working when somebody else ships.
    #[serde(other)]
    Unknown,
}

impl Message {
    /// The `Type` string, for a log line that wants to say what arrived.
    pub fn type_name(&self) -> &'static str {
        match self {
            Message::Join { .. } => "MVR_JOIN",
            Message::JoinRet { .. } => "MVR_JOIN_RET",
            Message::Leave { .. } => "MVR_LEAVE",
            Message::LeaveRet { .. } => "MVR_LEAVE_RET",
            Message::Commit(_) => "MVR_COMMIT",
            Message::CommitRet { .. } => "MVR_COMMIT_RET",
            Message::Request { .. } => "MVR_REQUEST",
            Message::RequestRet { .. } => "MVR_REQUEST_RET",
            Message::NewSessionHost { .. } => "MVR_NEW_SESSION_HOST",
            Message::NewSessionHostRet { .. } => "MVR_NEW_SESSION_HOST_RET",
            Message::Unknown => "unknown",
        }
    }

    /// The answer this console owes a message it cannot otherwise act on, so a
    /// caller never leaves a peer waiting on a connection it opened.
    ///
    /// `None` for the `*_RET` messages and for [`Message::Unknown`]: answering an
    /// answer is how two implementations talk to each other for ever.
    pub fn refusal(&self, why: &str) -> Option<Message> {
        let response = Response::failed(why);
        Some(match self {
            Message::Join { .. } => Message::JoinRet {
                response,
                provider: String::new(),
                ver_major: 0,
                ver_minor: 0,
                station_uuid: Uuid::nil(),
                station_name: String::new(),
                commits: Vec::new(),
            },
            Message::Leave { .. } => Message::LeaveRet { response },
            Message::Commit(_) => Message::CommitRet { response },
            Message::Request { .. } => Message::RequestRet { response },
            Message::NewSessionHost { .. } => Message::NewSessionHostRet { response },
            _ => return None,
        })
    }

    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Message, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// Read a uuid written either bare or as the first element of an array.
///
/// `MVR_REQUEST`'s `FromStationUUID` is typed as an array in Table 74 and written as a
/// string in the example beneath it. An empty string is `None`, since that is how the
/// example spells "no particular station".
fn one_or_many<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Uuid>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<Uuid>),
        Null,
    }

    Ok(match OneOrMany::deserialize(de)? {
        OneOrMany::One(s) if s.is_empty() => None,
        OneOrMany::One(s) => Uuid::parse_str(&s).ok(),
        OneOrMany::Many(v) => v.first().copied(),
        OneOrMany::Null => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leave_is_read_from_either_spelling() {
        let table = br#"{"Type":"MVR_LEAVE","FromStationUUID":"4aa291a1-1a62-45fe-aabc-e90e5e2399a8"}"#;
        let example = br#"{"Type":"MVR_LEAVE","StationUUID":"4aa291a1-1a62-45fe-aabc-e90e5e2399a8"}"#;
        assert_eq!(Message::from_json(table).unwrap(), Message::from_json(example).unwrap());
    }

    #[test]
    fn a_request_reads_a_station_as_a_string_or_an_array() {
        let one = br#"{"Type":"MVR_REQUEST","FromStationUUID":"4aa291a1-1a62-45fe-aabc-e90e5e2399a8"}"#;
        let many = br#"{"Type":"MVR_REQUEST","FromStationUUID":["4aa291a1-1a62-45fe-aabc-e90e5e2399a8"]}"#;
        assert_eq!(Message::from_json(one).unwrap(), Message::from_json(many).unwrap());
    }

    #[test]
    fn an_empty_station_in_a_request_is_no_station() {
        let empty = br#"{"Type":"MVR_REQUEST","FromStationUUID":"","FileUUID":null}"#;
        let Message::Request { from_station, file_uuid } = Message::from_json(empty).unwrap() else {
            panic!("not a request");
        };
        assert_eq!(from_station, None);
        assert_eq!(file_uuid, None);
    }

    #[test]
    fn a_message_type_from_the_future_is_kept_rather_than_refused() {
        let unknown = br#"{"Type":"MVR_SOMETHING_ELSE","Whatever":1}"#;
        assert_eq!(Message::from_json(unknown).unwrap(), Message::Unknown);
    }

    #[test]
    fn an_answer_is_never_answered() {
        assert!(Message::CommitRet { response: Response::ok() }.refusal("no").is_none());
        assert!(Message::Unknown.refusal("no").is_none());
        assert!(Message::Request { file_uuid: None, from_station: None }.refusal("no").is_some());
    }

    #[test]
    fn a_commit_addressed_to_others_is_not_ours() {
        let mine = Uuid::from_u128(1);
        let theirs = Uuid::from_u128(2);
        let mut commit = Commit {
            ver_major: 1,
            ver_minor: 6,
            file_size: 10,
            file_uuid: Uuid::from_u128(9),
            station_uuid: theirs,
            for_stations: vec![],
            comment: String::new(),
            file_name: String::new(),
        };
        assert!(commit.is_for(mine), "an empty list is everybody");
        commit.for_stations = vec![theirs];
        assert!(!commit.is_for(mine));
        commit.for_stations = vec![theirs, mine];
        assert!(commit.is_for(mine));
    }
}
