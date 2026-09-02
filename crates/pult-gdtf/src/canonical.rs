//! One spelling of a document, so two of them can be compared.
//!
//! XML has many ways to say the same thing: attribute order, self-closing versus
//! empty pairs, indentation, `1` against `1.000000`. A round-trip test that compared
//! raw text would fail on all of them and prove nothing, so both sides are put
//! through here first.
//!
//! What it deliberately does *not* do is reorder children. Element order is
//! meaningful in both these formats — a mode's channels are a sequence — and a
//! canonicaliser that sorted them would hide exactly the bug it exists to catch. Our
//! writer emits children in the order the object model declares them, which is the
//! spec's; a hand-authored fixture file has to be written in that order too.

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use crate::Error;

/// Rewrite a document into its canonical form.
pub fn canonicalize(xml: &str) -> Result<String, Error> {
    let mut reader = Reader::from_str(xml.trim_start_matches('\u{feff}'));
    let config = reader.config_mut();
    config.trim_text(true);
    config.expand_empty_elements = true;

    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    loop {
        match reader.read_event()? {
            Event::Eof => break,
            // The declaration says nothing about the content; dropping it means a
            // file written with `standalone="yes"` and one without compare equal.
            Event::Decl(_) | Event::Comment(_) => {}
            Event::Start(start) => {
                writer.write_event(Event::Start(canonical_start(&start)?))?;
            }
            Event::End(end) => writer.write_event(Event::End(end))?,
            Event::Empty(start) => {
                writer.write_event(Event::Empty(canonical_start(&start)?))?;
            }
            // `trim_text` has already dropped the whitespace-only ones, so what is
            // left is content and goes through untouched.
            Event::Text(text) => writer.write_event(Event::Text(text))?,
            other => writer.write_event(other)?,
        }
    }

    Ok(String::from_utf8(writer.into_inner()).expect("quick-xml writes utf-8"))
}

/// One element's attributes, sorted by name, with numbers normalised.
fn canonical_start(start: &BytesStart<'_>) -> Result<BytesStart<'static>, Error> {
    let name = start.name().as_ref().to_string();
    let mut attributes: Vec<(String, String)> = Vec::new();
    for attribute in start.attributes() {
        let attribute = attribute?;
        let key = attribute.key.as_ref().to_string();
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::Explicit1_0)?
            .into_owned();
        // An attribute the spec calls optional and a writer left empty is the same
        // as one that is not there, and files disagree about which they write.
        if value.is_empty() {
            continue;
        }
        attributes.push((key, normalise(&value)));
    }
    attributes.sort();

    let mut out = BytesStart::new(name);
    for (key, value) in attributes {
        out.push_attribute((key.as_str(), value.as_str()));
    }
    Ok(out)
}

/// `1.000000` and `1` are the same number; `{1.0,0.0}` and `{1,0}` are the same
/// matrix. Applied to every comma- and brace-separated part of a value, so the
/// structured strings normalise without being parsed as their own types.
fn normalise(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut token = String::new();
    for ch in value.chars() {
        if matches!(ch, ',' | '{' | '}' | ' ') {
            push_token(&mut out, &token);
            token.clear();
            if ch != ' ' {
                out.push(ch);
            }
        } else {
            token.push(ch);
        }
    }
    push_token(&mut out, &token);
    out
}

fn push_token(out: &mut String, token: &str) {
    if token.is_empty() {
        return;
    }
    match token.parse::<f64>() {
        // Only a token that is *entirely* a number, so `255/1` and `Gobo 3` pass
        // through untouched.
        Ok(number) => out.push_str(&crate::values::num(number as f32)),
        Err(_) => out.push_str(token),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_order_and_number_spelling_do_not_count() {
        let a = r#"<A Beta="1.000000" Alpha="x"><B Pos="{1.0,0.0,0.0}"/></A>"#;
        let b = "<A Alpha=\"x\" Beta=\"1\" Gamma=\"\">\n  <B Pos=\"{1,0,0}\"></B>\n</A>";
        assert_eq!(canonicalize(a).unwrap(), canonicalize(b).unwrap());
    }

    #[test]
    fn child_order_still_counts() {
        let a = "<A><B/><C/></A>";
        let b = "<A><C/><B/></A>";
        assert_ne!(canonicalize(a).unwrap(), canonicalize(b).unwrap());
    }
}
