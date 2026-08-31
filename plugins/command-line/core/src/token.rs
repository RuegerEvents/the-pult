//! Words, numbers, strings and the three symbols, with byte spans.

use crate::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    /// The text as the grammar reads it — quotes stripped, `@` spelled `at`.
    pub text: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TokenKind {
    Word,
    Number,
    /// A quoted string, quotes already stripped.
    Str,
    Plus,
    Minus,
}

/// Split a line into tokens.
///
/// A token is a maximal run of non-whitespace, except that a lone `+` or `-`
/// is a symbol — so `1 - 2` subtracts while a uuid with hyphens stays one
/// word. `@` is the classic console spelling of `at` and becomes exactly that,
/// leading digits attached or not: `@80` is `at 80`.
pub fn tokenize(line: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if bytes[i] == b'"' {
            // A string runs to the closing quote, or to the end of the line
            // while it is still being typed.
            i += 1;
            let text_start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            let text = line[text_start..i].to_string();
            if i < bytes.len() {
                i += 1;
            }
            tokens.push(Token { kind: TokenKind::Str, text, span: (start, i) });
            continue;
        }
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'"' {
            i += 1;
        }
        let raw = &line[start..i];
        match raw {
            "+" => tokens.push(Token { kind: TokenKind::Plus, text: "+".into(), span: (start, i) }),
            "-" => tokens.push(Token { kind: TokenKind::Minus, text: "-".into(), span: (start, i) }),
            _ => {
                if let Some(rest) = raw.strip_prefix('@') {
                    tokens.push(Token {
                        kind: TokenKind::Word,
                        text: "at".into(),
                        span: (start, start + 1),
                    });
                    if !rest.is_empty() {
                        tokens.push(classify(rest, (start + 1, i)));
                    }
                } else {
                    tokens.push(classify(raw, (start, i)));
                }
            }
        }
    }
    tokens
}

fn classify(raw: &str, span: Span) -> Token {
    let kind = if raw.parse::<f64>().is_ok() { TokenKind::Number } else { TokenKind::Word };
    Token { kind, text: raw.to_string(), span }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<(TokenKind, String)> {
        tokenize(line).into_iter().map(|t| (t.kind, t.text)).collect()
    }

    #[test]
    fn the_classic_spellings_read_the_same() {
        assert_eq!(kinds("fixture 1 @ 80"), kinds("fixture 1 at 80"));
        assert_eq!(kinds("@80"), kinds("at 80"));
    }

    #[test]
    fn a_lone_minus_subtracts_but_a_uuid_stays_whole() {
        let uuid = "2f6b535b-9a71-4c39-9d95-6d6ab2f0f639";
        assert_eq!(kinds(uuid), vec![(TokenKind::Word, uuid.to_string())]);
        assert_eq!(
            kinds("1 - 2"),
            vec![
                (TokenKind::Number, "1".into()),
                (TokenKind::Minus, "-".into()),
                (TokenKind::Number, "2".into()),
            ]
        );
    }

    #[test]
    fn spans_are_byte_offsets_into_the_line() {
        let tokens = tokenize(r#"rename sequence 2 "Songs about lights""#);
        let last = tokens.last().unwrap();
        assert_eq!(last.kind, TokenKind::Str);
        assert_eq!(last.text, "Songs about lights");
        assert_eq!(last.span, (18, 38));
    }

    #[test]
    fn an_unterminated_string_is_the_rest_of_the_line() {
        let tokens = tokenize(r#"rename sequence 2 "Son"#);
        assert_eq!(tokens.last().unwrap().text, "Son");
    }
}
