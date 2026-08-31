//! From tokens to a [`Command`], or to an error that points at the problem.

use serde_json::Value;

use crate::catalog::Catalog;
use crate::token::{tokenize, Token, TokenKind};
use crate::{Command, ParseError, Range, SelOp, Span, Target};

pub fn parse(catalog: &Catalog, line: &str) -> Result<Command, ParseError> {
    let tokens = tokenize(line);
    if tokens.is_empty() {
        return Err(ParseError::new("nothing to do", (0, 0)));
    }
    let mut p = Parser { catalog, tokens: &tokens, pos: 0, line_len: line.len() };
    let command = p.command()?;
    if let Some(extra) = p.peek() {
        return Err(ParseError::new(
            format!("did not expect {:?} here", extra.text),
            extra.span,
        ));
    }
    Ok(command)
}

struct Parser<'a> {
    catalog: &'a Catalog,
    tokens: &'a [Token],
    pos: usize,
    line_len: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<&'a Token> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    /// Where an error about "something missing here" points: after the last
    /// token, not at position zero.
    fn end_span(&self) -> Span {
        (self.line_len, self.line_len)
    }

    fn command(&mut self) -> Result<Command, ParseError> {
        let first = self.next().expect("parse() checked for emptiness");
        let word = first.text.to_lowercase();
        match word.as_str() {
            "help" => {
                let topic = self.next().map(|t| t.text.clone());
                Ok(Command::Help { topic })
            }
            "clear" => {
                let also_selection = match self.peek() {
                    Some(t) if t.text.eq_ignore_ascii_case("clear") => {
                        self.next();
                        true
                    }
                    _ => false,
                };
                Ok(Command::Clear { also_selection })
            }
            "at" => {
                let percent = self.number("a level, 0 to 100")?;
                Ok(Command::Intensity { percent })
            }
            "full" => Ok(Command::Intensity { percent: 100.0 }),
            "out" => Ok(Command::Intensity { percent: 0.0 }),
            "create" => {
                let table = self.table(first.span)?;
                let name = match self.peek() {
                    Some(t) if t.kind == TokenKind::Str => Some(self.next().unwrap().text.clone()),
                    _ => None,
                };
                Ok(Command::Create { table, name })
            }
            "delete" => {
                let table = self.table(first.span)?;
                let target = self.target()?;
                Ok(Command::Delete { table, target })
            }
            "rename" => {
                let table = self.table(first.span)?;
                let target = self.target()?;
                let name = self.string("the new name, in quotes")?;
                Ok(Command::SetField {
                    table,
                    target,
                    field: "name".into(),
                    value: Value::String(name),
                })
            }
            "set" => {
                let table = self.table(first.span)?;
                let target = self.target()?;
                let field = self.field_of(&table)?;
                let value = self.scalar("a value")?;
                Ok(Command::SetField { table, target, field, value })
            }
            "store" => {
                self.keyword("sequence")?;
                let sequence = self.target()?;
                self.keyword("cue")?;
                let cue = self.target()?;
                Ok(Command::Store { sequence, cue })
            }
            _ => {
                if self.catalog.rpc_prefixes().iter().any(|p| *p == word) {
                    return self.rpc(&word, first.span);
                }
                if let Some(entity) = self.catalog.table_for(&word) {
                    let table = entity.table.clone();
                    if table == "fixtures" {
                        return self.selection(first.span);
                    }
                    return self.entity_command(table);
                }
                Err(ParseError::new(
                    format!("{:?} is not a command here", first.text),
                    first.span,
                )
                .expecting(self.first_words()))
            }
        }
    }

    /// What could begin a line, for the error under an unknown first word.
    fn first_words(&self) -> Vec<String> {
        let mut words: Vec<String> = ["help", "clear", "at", "full", "out", "create", "delete",
            "rename", "set", "store"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        words.extend(self.catalog.entity_words());
        words.extend(self.catalog.rpc_prefixes());
        words
    }

    // ── Pieces ────────────────────────────────────────────────────────────────

    fn table(&mut self, after: Span) -> Result<String, ParseError> {
        match self.next() {
            Some(t) => match self.catalog.table_for(&t.text) {
                Some(entity) if !entity.is_singleton => Ok(entity.table.clone()),
                Some(_) => Err(ParseError::new(
                    format!("{:?} is the whole show, not a collection", t.text),
                    t.span,
                )),
                None => Err(ParseError::new(format!("no collection called {:?}", t.text), t.span)
                    .expecting(self.catalog.entity_words())),
            },
            None => Err(ParseError::new("which collection?", (after.1, after.1))
                .expecting(self.catalog.entity_words())),
        }
    }

    fn target(&mut self) -> Result<Target, ParseError> {
        match self.next() {
            Some(t) if t.kind == TokenKind::Number => {
                let n = t.text.parse::<usize>().map_err(|_| {
                    ParseError::new("a whole number names an entry", t.span)
                })?;
                if n == 0 {
                    return Err(ParseError::new("numbering starts at 1", t.span));
                }
                Ok(Target::Index(n))
            }
            Some(t) if t.kind == TokenKind::Str => Ok(Target::Name(t.text.clone())),
            Some(t) => Err(ParseError::new(
                format!("{:?} does not name an entry", t.text),
                t.span,
            )
            .expecting(vec!["a number".into(), "\"a name\"".into()])),
            None => Err(ParseError::new("which one?", self.end_span())
                .expecting(vec!["a number".into(), "\"a name\"".into()])),
        }
    }

    fn field_of(&mut self, table: &str) -> Result<String, ParseError> {
        let fields: Vec<String> = self
            .catalog
            .entities
            .iter()
            .find(|e| e.table == table)
            .map(|e| e.fields.clone())
            .unwrap_or_default();
        match self.next() {
            Some(t) => {
                let wanted = crate::catalog::normal(&t.text);
                fields
                    .iter()
                    .find(|f| crate::catalog::normal(f) == wanted)
                    .cloned()
                    .ok_or_else(|| {
                        ParseError::new(format!("{} has no field {:?}", table, t.text), t.span)
                            .expecting(fields.clone())
                    })
            }
            None => Err(ParseError::new("which field?", self.end_span()).expecting(fields)),
        }
    }

    fn number(&mut self, what: &str) -> Result<f64, ParseError> {
        match self.next() {
            Some(t) if t.kind == TokenKind::Number => Ok(t.text.parse().unwrap_or(0.0)),
            Some(t) => Err(ParseError::new(format!("expected {what}"), t.span)),
            None => Err(ParseError::new(format!("expected {what}"), self.end_span())),
        }
    }

    fn string(&mut self, what: &str) -> Result<String, ParseError> {
        match self.next() {
            Some(t) if t.kind == TokenKind::Str => Ok(t.text.clone()),
            Some(t) => Err(ParseError::new(format!("expected {what}"), t.span)),
            None => Err(ParseError::new(format!("expected {what}"), self.end_span())),
        }
    }

    fn keyword(&mut self, word: &str) -> Result<(), ParseError> {
        match self.next() {
            Some(t) if t.text.eq_ignore_ascii_case(word) => Ok(()),
            Some(t) => Err(ParseError::new(format!("expected {word:?}"), t.span)
                .expecting(vec![word.into()])),
            None => Err(ParseError::new(format!("expected {word:?}"), self.end_span())
                .expecting(vec![word.into()])),
        }
    }

    /// One value: a number, a quoted string, on/off/true/false, or a bare word.
    fn scalar(&mut self, what: &str) -> Result<Value, ParseError> {
        // A minus right before a number is that number, negative.
        if matches!(self.peek(), Some(t) if t.kind == TokenKind::Minus) {
            let minus = self.next().unwrap();
            let n = self.number("a number")?;
            let _ = minus;
            return Ok(number_value(-n));
        }
        match self.next() {
            Some(t) => Ok(match t.kind {
                TokenKind::Number => number_value(t.text.parse().unwrap_or(0.0)),
                TokenKind::Str => Value::String(t.text.clone()),
                _ => match t.text.to_lowercase().as_str() {
                    "true" | "on" => Value::Bool(true),
                    "false" | "off" => Value::Bool(false),
                    "null" | "none" => Value::Null,
                    _ => Value::String(t.text.clone()),
                },
            }),
            None => Err(ParseError::new(format!("expected {what}"), self.end_span())),
        }
    }

    /// `1 thru 5 + 7 - 2 thru 3`, after the word `fixture`.
    fn selection(&mut self, word_span: Span) -> Result<Command, ParseError> {
        let mut ops: Vec<(SelOp, Range)> = Vec::new();
        // The first bare range replaces; every one after it adds, the way
        // `fixture 1 3 5` reads aloud.
        let mut bare_op = SelOp::Replace;
        loop {
            match self.peek() {
                None => break,
                Some(t) if t.kind == TokenKind::Plus => {
                    self.next();
                    ops.push((SelOp::Add, self.required_range()?));
                    bare_op = SelOp::Add;
                }
                Some(t) if t.kind == TokenKind::Minus => {
                    self.next();
                    ops.push((SelOp::Remove, self.required_range()?));
                    bare_op = SelOp::Add;
                }
                Some(t) if t.kind == TokenKind::Number => {
                    ops.push((bare_op, self.range()?));
                    bare_op = SelOp::Add;
                }
                // `fixture 1 thru 3 @ 80` is one line on every console; the
                // selection ends where the level begins.
                Some(t) if is_level_word(&t.text) => break,
                Some(t) => {
                    return Err(ParseError::new(
                        format!("{:?} is not part of a selection", t.text),
                        t.span,
                    )
                    .expecting(vec![
                        "a number".into(),
                        "thru".into(),
                        "+".into(),
                        "-".into(),
                        "at".into(),
                        "full".into(),
                        "out".into(),
                    ]));
                }
            }
        }
        if ops.is_empty() {
            return Err(ParseError::new(
                "which fixtures?",
                (word_span.1, word_span.1),
            )
            .expecting(vec!["a number".into(), "1 thru 5".into()]));
        }
        let at = match self.peek() {
            Some(t) if t.text.eq_ignore_ascii_case("at") => {
                self.next();
                Some(self.number("a level, 0 to 100")?)
            }
            Some(t) if t.text.eq_ignore_ascii_case("full") => {
                self.next();
                Some(100.0)
            }
            Some(t) if t.text.eq_ignore_ascii_case("out") => {
                self.next();
                Some(0.0)
            }
            _ => None,
        };
        Ok(Command::Select { ops, at })
    }

    /// A range that has to be there: after a `+` or `-`.
    fn required_range(&mut self) -> Result<Range, ParseError> {
        match self.peek() {
            Some(t) if t.kind == TokenKind::Number => self.range(),
            Some(t) => Err(ParseError::new("a number has to follow", t.span)),
            None => Err(ParseError::new("a number has to follow", self.end_span())),
        }
    }

    fn range(&mut self) -> Result<Range, ParseError> {
        let from_token = self.next().expect("caller peeked a number");
        let from = from_token
            .text
            .parse::<usize>()
            .map_err(|_| ParseError::new("fixtures are numbered with whole numbers", from_token.span))?;
        if from == 0 {
            return Err(ParseError::new("numbering starts at 1", from_token.span));
        }
        if matches!(self.peek(), Some(t) if t.text.eq_ignore_ascii_case("thru")) {
            self.next();
            let to_token = self.next().ok_or_else(|| {
                ParseError::new("thru needs a number after it", self.end_span())
            })?;
            let to = to_token
                .text
                .parse::<usize>()
                .map_err(|_| ParseError::new("thru needs a number after it", to_token.span))?;
            if to < from {
                return Err(ParseError::new("a range runs upward", to_token.span));
            }
            return Ok(Range { from, to });
        }
        Ok(Range { from, to: from })
    }

    /// `sequence 2 go [args]` — target, then a registered command.
    fn entity_command(&mut self, table: String) -> Result<Command, ParseError> {
        let target = self.target()?;
        let commands: Vec<String> =
            self.catalog.commands_of(&table).map(|c| c.name.clone()).collect();
        let Some(word) = self.next() else {
            return Err(ParseError::new("and do what with it?", self.end_span())
                .expecting(commands));
        };
        let Some(info) = self.catalog.command_for(&table, &word.text) else {
            return Err(ParseError::new(
                format!("{} has no command {:?}", table, word.text),
                word.span,
            )
            .expecting(commands));
        };
        let name = info.name.clone();
        let arg_infos = info.args.clone();
        let mut args = Vec::new();
        for arg in &arg_infos {
            if self.peek().is_none() {
                break;
            }
            let value = self.scalar(&arg.name)?;
            args.push((arg.name.clone(), value));
        }
        Ok(Command::EntityCommand { table, target, command: name, args })
    }

    /// `session join <id>` — the method's declared args, in order.
    fn rpc(&mut self, prefix: &str, prefix_span: Span) -> Result<Command, ParseError> {
        let methods: Vec<String> = self
            .catalog
            .rpcs
            .iter()
            .filter_map(|r| r.method.strip_prefix(&format!("{prefix}.")).map(String::from))
            .collect();
        let Some(word) = self.next() else {
            return Err(ParseError::new(
                format!("{prefix} what?"),
                (prefix_span.1, prefix_span.1),
            )
            .expecting(methods));
        };
        let Some(info) = self.catalog.rpc_for(prefix, &word.text.to_lowercase()) else {
            return Err(ParseError::new(
                format!("no {prefix} command called {:?}", word.text),
                word.span,
            )
            .expecting(methods));
        };
        let method = info.method.clone();
        let arg_infos = info.args.clone();
        let mut args = Vec::new();
        for arg in &arg_infos {
            if self.peek().is_none() {
                if arg.optional {
                    continue;
                }
                return Err(ParseError::new(
                    format!("{} needs {}", method, arg.name),
                    self.end_span(),
                ));
            }
            let value = self.scalar(&arg.name)?;
            args.push((arg.name.clone(), value));
        }
        Ok(Command::Rpc { method, args })
    }
}

fn is_level_word(word: &str) -> bool {
    word.eq_ignore_ascii_case("at")
        || word.eq_ignore_ascii_case("full")
        || word.eq_ignore_ascii_case("out")
}

fn number_value(n: f64) -> Value {
    // Whole numbers as integers: an id or an index serialized as `3.0` would
    // fail the engine's validation for integer fields.
    if n.fract() == 0.0 && n.abs() < i64::MAX as f64 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::test_catalog;

    fn parse_ok(line: &str) -> Command {
        parse(&test_catalog(), line).unwrap_or_else(|e| panic!("{line:?} failed: {e:?}"))
    }

    #[test]
    fn the_opening_example_of_every_console_manual() {
        assert_eq!(
            parse_ok("fixture 1 thru 5"),
            Command::Select { ops: vec![(SelOp::Replace, Range { from: 1, to: 5 })], at: None }
        );
        assert_eq!(parse_ok("at 80"), Command::Intensity { percent: 80.0 });
        assert_eq!(parse_ok("@ 80"), Command::Intensity { percent: 80.0 });
        assert_eq!(parse_ok("full"), Command::Intensity { percent: 100.0 });
        // And the combined form, which is how the line is actually typed.
        assert_eq!(
            parse_ok("fixture 1 thru 3 @ 80"),
            Command::Select {
                ops: vec![(SelOp::Replace, Range { from: 1, to: 3 })],
                at: Some(80.0)
            }
        );
        assert_eq!(
            parse_ok("fixture 2 full"),
            Command::Select { ops: vec![(SelOp::Replace, Range { from: 2, to: 2 })], at: Some(100.0) }
        );
    }

    #[test]
    fn a_selection_is_edited_with_plus_and_minus() {
        assert_eq!(
            parse_ok("fixture 1 thru 5 + 7 - 2"),
            Command::Select {
                ops: vec![
                    (SelOp::Replace, Range { from: 1, to: 5 }),
                    (SelOp::Add, Range { from: 7, to: 7 }),
                    (SelOp::Remove, Range { from: 2, to: 2 }),
                ],
                at: None
            }
        );
        // Starting with + keeps what is already selected.
        assert_eq!(
            parse_ok("fixture + 9"),
            Command::Select { ops: vec![(SelOp::Add, Range { from: 9, to: 9 })], at: None }
        );
    }

    #[test]
    fn entity_commands_come_from_the_catalog_not_the_parser() {
        assert_eq!(
            parse_ok("sequence 2 go"),
            Command::EntityCommand {
                table: "sequences".into(),
                target: Target::Index(2),
                command: "goNext".into(),
                args: vec![],
            }
        );
        assert_eq!(
            parse_ok("sequence 2 goto 3"),
            Command::EntityCommand {
                table: "sequences".into(),
                target: Target::Index(2),
                command: "goToCue".into(),
                args: vec![("cueId".into(), Value::from(3))],
            }
        );
    }

    #[test]
    fn the_editing_verbs() {
        assert_eq!(
            parse_ok(r#"create sequence "Chases""#),
            Command::Create { table: "sequences".into(), name: Some("Chases".into()) }
        );
        assert_eq!(
            parse_ok("delete fixture 3"),
            Command::Delete { table: "fixtures".into(), target: Target::Index(3) }
        );
        assert_eq!(
            parse_ok(r#"rename sequence 2 "Songs""#),
            Command::SetField {
                table: "sequences".into(),
                target: Target::Index(2),
                field: "name".into(),
                value: Value::String("Songs".into()),
            }
        );
        assert_eq!(
            parse_ok("set cue 3 fade_time 4"),
            Command::SetField {
                table: "cues".into(),
                target: Target::Index(3),
                field: "fade_time".into(),
                value: Value::from(4),
            }
        );
        assert_eq!(
            parse_ok("store sequence 2 cue 3"),
            Command::Store { sequence: Target::Index(2), cue: Target::Index(3) }
        );
    }

    #[test]
    fn rpcs_read_their_arguments_from_the_schema() {
        assert_eq!(
            parse_ok("device adopt 4d5e6f"),
            Command::Rpc {
                method: "device.adopt".into(),
                args: vec![("serial".into(), Value::String("4d5e6f".into()))],
            }
        );
    }

    #[test]
    fn errors_point_at_the_problem_and_name_the_alternatives() {
        let catalog = test_catalog();

        let err = parse(&catalog, "sequence 2 fly").unwrap_err();
        assert_eq!(err.span, (11, 14));
        assert!(err.expected.contains(&"goNext".to_string()), "{err:?}");

        let err = parse(&catalog, "set cue 3 fade 4").unwrap_err();
        assert!(err.message.contains("no field"), "{err:?}");
        assert!(err.expected.contains(&"fade_time".to_string()));

        let err = parse(&catalog, "banish fixture 3").unwrap_err();
        assert_eq!(err.span.0, 0);
        assert!(err.expected.contains(&"delete".to_string()));

        // A range that runs downward is named, not silently emptied.
        let err = parse(&catalog, "fixture 5 thru 2").unwrap_err();
        assert!(err.message.contains("upward"));
    }

    #[test]
    fn field_spelling_is_forgiving_but_the_ast_is_exact() {
        assert_eq!(
            parse_ok("set cue 3 fadetime 4"),
            parse_ok("set cue 3 fade_time 4")
        );
    }
}
