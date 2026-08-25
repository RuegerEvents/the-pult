use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

// Variant order matters for `#[serde(untagged)]` deserialization:
// serde tries each variant top-to-bottom. `Id(Uuid)` must come before `Key(String)`
// so that UUID strings like "2f6b535b-…" deserialize as Id, not as Key.
// Non-UUID strings (field names) fall through to Key. Numbers go to Index.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(untagged)]
pub enum PathSegment {
    Id(Uuid),
    Index(usize),
    Key(String),
}

impl From<&str> for PathSegment {
    fn from(s: &str) -> Self {
        PathSegment::Key(s.to_owned())
    }
}

impl From<String> for PathSegment {
    fn from(s: String) -> Self {
        PathSegment::Key(s)
    }
}

impl From<usize> for PathSegment {
    fn from(n: usize) -> Self {
        PathSegment::Index(n)
    }
}

impl From<Uuid> for PathSegment {
    fn from(id: Uuid) -> Self {
        PathSegment::Id(id)
    }
}

impl std::fmt::Display for PathSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathSegment::Key(k) => write!(f, "{k}"),
            PathSegment::Index(n) => write!(f, "{n}"),
            PathSegment::Id(id) => write!(f, "{id}"),
        }
    }
}

pub type Path = Vec<PathSegment>;

pub fn path_key(mut path: Path, key: &str) -> Path {
    path.push(PathSegment::Key(key.to_owned()));
    path
}

pub fn path_index(mut path: Path, index: usize) -> Path {
    path.push(PathSegment::Index(index));
    path
}

pub fn path_id(mut path: Path, id: Uuid) -> Path {
    path.push(PathSegment::Id(id));
    path
}

/// Glob-style pattern for subscriptions. `*` = one segment, `**` = any number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PathPattern(pub String);

impl PathPattern {
    pub fn new(pattern: impl Into<String>) -> Self {
        Self(pattern.into())
    }

    pub fn matches(&self, path: &Path) -> bool {
        let parts: Vec<&str> = self.0.split('/').collect();
        match_pattern(&parts, path)
    }
}

fn match_pattern(pattern: &[&str], path: &[PathSegment]) -> bool {
    match (pattern.first(), path.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            for skip in 0..=path.len() {
                if match_pattern(&pattern[1..], &path[skip..]) {
                    return true;
                }
            }
            false
        }
        (Some(&"*"), Some(_)) => match_pattern(&pattern[1..], &path[1..]),
        (Some(p), Some(seg)) => {
            if seg.to_string() == *p {
                match_pattern(&pattern[1..], &path[1..])
            } else {
                false
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_exact() {
        let p = PathPattern::new("sequences/0/name");
        let path = vec![
            PathSegment::Key("sequences".into()),
            PathSegment::Index(0),
            PathSegment::Key("name".into()),
        ];
        assert!(p.matches(&path));
    }

    #[test]
    fn pattern_single_wildcard() {
        let p = PathPattern::new("sequences/*/name");
        let path = vec![
            PathSegment::Key("sequences".into()),
            PathSegment::Index(5),
            PathSegment::Key("name".into()),
        ];
        assert!(p.matches(&path));
    }

    #[test]
    fn pattern_double_wildcard() {
        let p = PathPattern::new("sequences/**");
        let path = vec![
            PathSegment::Key("sequences".into()),
            PathSegment::Index(5),
            PathSegment::Key("cues".into()),
            PathSegment::Index(3),
            PathSegment::Key("fadeTime".into()),
        ];
        assert!(p.matches(&path));
    }
}
