//! What could come next, given a line and a cursor.
//!
//! Everything words alone can answer is answered here: keywords, verbs, table
//! names, command names, field names. What needs the show — which sequences
//! exist, what they are called — comes back as a symbolic [`Expectation`] for
//! the plugin to fill in from data, so this stays pure and the grammar's
//! knowledge stays in one crate.

use crate::catalog::Catalog;
use crate::token::{tokenize, Token, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub struct Completions {
    /// Byte offset the chosen completion replaces from (up to the cursor).
    pub replace_from: usize,
    /// What is already typed of the current word, for filtering.
    pub prefix: String,
    pub expectations: Vec<Expectation>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expectation {
    /// A word to type, with a note for beside it.
    Keyword { word: String, detail: String },
    /// An entry of this collection: the plugin offers numbers and names.
    EntityRef { table: String },
    /// A free value; the hint says what kind.
    Value { hint: String },
}

fn keyword(word: impl Into<String>, detail: impl Into<String>) -> Expectation {
    Expectation::Keyword { word: word.into(), detail: detail.into() }
}

pub fn complete(catalog: &Catalog, line: &str, cursor: usize) -> Completions {
    let cursor = cursor.min(line.len());
    let all = tokenize(&line[..cursor]);

    // The word being typed is the token touching the cursor; everything before
    // it is settled and decides the state.
    let (settled, partial): (&[Token], Option<&Token>) = match all.last() {
        Some(last) if last.span.1 == cursor && last.kind != TokenKind::Str => {
            (&all[..all.len() - 1], Some(last))
        }
        _ => (&all[..], None),
    };
    let replace_from = partial.map(|t| t.span.0).unwrap_or(cursor);
    let prefix = partial.map(|t| t.text.clone()).unwrap_or_default();

    let expectations = expectations_after(catalog, settled);
    Completions { replace_from, prefix, expectations }
}

/// The grammar's answer to "and now what?", walked over the settled tokens.
fn expectations_after(catalog: &Catalog, tokens: &[Token]) -> Vec<Expectation> {
    let Some(first) = tokens.first() else {
        return first_words(catalog);
    };
    let word = first.text.to_lowercase();
    let rest = &tokens[1..];
    match word.as_str() {
        "help" => {
            if rest.is_empty() {
                let mut topics = vec![keyword("selection", "selecting fixtures")];
                topics.extend(
                    catalog.entity_words().into_iter().map(|w| keyword(w, "a collection")),
                );
                topics
            } else {
                Vec::new()
            }
        }
        "clear" => {
            if rest.is_empty() {
                vec![keyword("clear", "also drop the selection")]
            } else {
                Vec::new()
            }
        }
        "at" => {
            if rest.is_empty() {
                vec![Expectation::Value { hint: "a level, 0 to 100".into() }]
            } else {
                Vec::new()
            }
        }
        "full" | "out" => Vec::new(),
        "create" => match rest.len() {
            0 => tables(catalog),
            1 => vec![Expectation::Value { hint: "\"a name\"".into() }],
            _ => Vec::new(),
        },
        "delete" => match rest.len() {
            0 => tables(catalog),
            1 => entity_ref(catalog, &rest[0]),
            _ => Vec::new(),
        },
        "rename" => match rest.len() {
            0 => tables(catalog),
            1 => entity_ref(catalog, &rest[0]),
            2 => vec![Expectation::Value { hint: "\"the new name\"".into() }],
            _ => Vec::new(),
        },
        "set" => match rest.len() {
            0 => tables(catalog),
            1 => entity_ref(catalog, &rest[0]),
            2 => fields(catalog, &rest[0]),
            3 => vec![Expectation::Value { hint: "a value".into() }],
            _ => Vec::new(),
        },
        "store" => match rest.len() {
            0 => vec![keyword("sequence", "which sequence to store into")],
            1 => vec![Expectation::EntityRef { table: "sequences".into() }],
            2 => vec![keyword("cue", "which cue to store as")],
            3 => vec![Expectation::EntityRef { table: "cues".into() }],
            _ => Vec::new(),
        },
        _ => {
            if catalog.rpc_prefixes().contains(&word) {
                return rpc_expectations(catalog, &word, rest);
            }
            let Some(entity) = catalog.table_for(&word) else {
                return Vec::new();
            };
            let table = entity.table.clone();
            if table == "fixtures" {
                return selection_expectations(rest);
            }
            match rest.len() {
                0 => entity_ref_direct(&table),
                1 => catalog
                    .commands_of(&table)
                    .map(|c| keyword(&c.name, first_sentence(&c.doc)))
                    .collect(),
                n => {
                    // Inside a command's arguments: hint the next declared one.
                    let Some(info) = catalog.command_for(&table, &rest[1].text) else {
                        return Vec::new();
                    };
                    info.args
                        .get(n - 2)
                        .map(|arg| {
                            vec![Expectation::Value {
                                hint: format!(
                                    "{}: {}{}",
                                    arg.name,
                                    arg.ty,
                                    if arg.optional { " (optional)" } else { "" }
                                ),
                            }]
                        })
                        .unwrap_or_default()
                }
            }
        }
    }
}

fn first_words(catalog: &Catalog) -> Vec<Expectation> {
    let mut words = vec![
        keyword("fixture", "select fixtures: fixture 1 thru 5"),
        keyword("at", "set intensity on the selection"),
        keyword("full", "selection to 100%"),
        keyword("out", "selection to 0%"),
        keyword("clear", "empty the programmer"),
        keyword("create", "add an entry to a collection"),
        keyword("delete", "remove an entry"),
        keyword("rename", "change an entry's name"),
        keyword("set", "change one field of an entry"),
        keyword("store", "programmer into a cue"),
        keyword("help", "how any of this works"),
    ];
    for table in catalog.entity_words() {
        if table != "fixture" {
            words.push(keyword(&table, "a collection"));
        }
    }
    for prefix in catalog.rpc_prefixes() {
        words.push(keyword(&prefix, "station commands"));
    }
    words
}

fn tables(catalog: &Catalog) -> Vec<Expectation> {
    catalog.entity_words().into_iter().map(|w| keyword(w, "a collection")).collect()
}

fn entity_ref(catalog: &Catalog, table_token: &Token) -> Vec<Expectation> {
    catalog
        .table_for(&table_token.text)
        .map(|e| vec![Expectation::EntityRef { table: e.table.clone() }])
        .unwrap_or_default()
}

fn entity_ref_direct(table: &str) -> Vec<Expectation> {
    vec![Expectation::EntityRef { table: table.to_string() }]
}

fn fields(catalog: &Catalog, table_token: &Token) -> Vec<Expectation> {
    catalog
        .table_for(&table_token.text)
        .map(|e| e.fields.iter().map(|f| keyword(f, "a field")).collect())
        .unwrap_or_default()
}

fn selection_expectations(rest: &[Token]) -> Vec<Expectation> {
    match rest.last() {
        None => vec![
            Expectation::EntityRef { table: "fixtures".into() },
            keyword("+", "add to the selection"),
            keyword("-", "take out of the selection"),
        ],
        Some(t) if t.kind == TokenKind::Number => vec![
            keyword("thru", "a range: 1 thru 5"),
            keyword("+", "add more"),
            keyword("-", "take some out"),
            keyword("at", "…and to a level: at 80"),
            keyword("full", "…and to 100%"),
            keyword("out", "…and to 0%"),
        ],
        Some(t) if t.kind == TokenKind::Plus || t.kind == TokenKind::Minus => {
            vec![Expectation::EntityRef { table: "fixtures".into() }]
        }
        Some(t) if t.text.eq_ignore_ascii_case("thru") => {
            vec![Expectation::Value { hint: "the end of the range".into() }]
        }
        Some(_) => Vec::new(),
    }
}

fn rpc_expectations(catalog: &Catalog, prefix: &str, rest: &[Token]) -> Vec<Expectation> {
    match rest.len() {
        0 => catalog
            .rpcs
            .iter()
            .filter_map(|r| {
                let method = r.method.strip_prefix(&format!("{prefix}."))?;
                Some(keyword(method, first_sentence(&r.doc)))
            })
            .collect(),
        n => {
            let Some(info) = catalog.rpc_for(prefix, &rest[0].text.to_lowercase()) else {
                return Vec::new();
            };
            info.args
                .get(n - 1)
                .map(|arg| vec![Expectation::Value { hint: format!("{}: {}", arg.name, arg.ty) }])
                .unwrap_or_default()
        }
    }
}

fn first_sentence(doc: &str) -> String {
    doc.split(['.', '\n']).next().unwrap_or("").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::test_catalog;

    fn words_at(line: &str, cursor: usize) -> Vec<String> {
        complete(&test_catalog(), line, cursor)
            .expectations
            .into_iter()
            .filter_map(|e| match e {
                Expectation::Keyword { word, .. } => Some(word),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_empty_line_offers_the_verbs_and_the_collections() {
        let words = words_at("", 0);
        for expected in ["fixture", "at", "clear", "sequence", "session", "help"] {
            assert!(words.contains(&expected.to_string()), "missing {expected} in {words:?}");
        }
    }

    #[test]
    fn a_half_typed_word_reports_its_own_start() {
        let c = complete(&test_catalog(), "seq", 3);
        assert_eq!(c.replace_from, 0);
        assert_eq!(c.prefix, "seq");
    }

    #[test]
    fn after_a_sequence_target_the_commands_appear_with_their_docs() {
        let c = complete(&test_catalog(), "sequence 2 ", 11);
        let words: Vec<_> = c
            .expectations
            .iter()
            .filter_map(|e| match e {
                Expectation::Keyword { word, detail } => Some((word.clone(), detail.clone())),
                _ => None,
            })
            .collect();
        assert!(words.iter().any(|(w, d)| w == "goNext" && d.contains("next cue")), "{words:?}");
    }

    #[test]
    fn what_needs_the_show_comes_back_symbolic() {
        let c = complete(&test_catalog(), "delete sequence ", 16);
        assert_eq!(c.expectations, vec![Expectation::EntityRef { table: "sequences".into() }]);
    }

    #[test]
    fn mid_selection_the_grammar_knows_where_it_is() {
        let words = words_at("fixture 1 ", 10);
        assert!(words.contains(&"thru".to_string()), "{words:?}");
        let c = complete(&test_catalog(), "fixture 1 thru 3 + ", 19);
        assert_eq!(c.expectations, vec![Expectation::EntityRef { table: "fixtures".into() }]);
    }

    #[test]
    fn set_walks_table_then_entry_then_field_then_value() {
        let words = words_at("set ", 4);
        assert!(words.contains(&"cue".to_string()));
        let words = words_at("set cue 3 ", 10);
        assert!(words.contains(&"fade_time".to_string()), "{words:?}");
        let c = complete(&test_catalog(), "set cue 3 fade_time ", 20);
        assert!(matches!(c.expectations[0], Expectation::Value { .. }));
    }

    #[test]
    fn rpc_methods_and_their_arguments_hint_themselves() {
        let words = words_at("session ", 8);
        assert_eq!(words, vec!["join".to_string()]);
        let c = complete(&test_catalog(), "session join ", 13);
        assert!(
            matches!(&c.expectations[0], Expectation::Value { hint } if hint.contains("sessionId"))
        );
    }
}
