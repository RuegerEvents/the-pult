//! Help text, assembled from the catalog rather than written twice.

use crate::catalog::{singular_of, Catalog};

pub fn help(catalog: &Catalog, topic: Option<&str>) -> String {
    match topic {
        None => overview(catalog),
        Some(word) => {
            let normal = word.to_lowercase();
            if normal == "selection" || normal == "fixture" {
                return SELECTION_HELP.trim().to_string();
            }
            if let Some(entity) = catalog.table_for(&normal) {
                return entity_help(catalog, &entity.table);
            }
            if catalog.rpc_prefixes().contains(&normal) {
                return rpc_help(catalog, &normal);
            }
            format!(
                "Nothing called {word:?} to explain. `help` alone lists what there is."
            )
        }
    }
}

const SELECTION_HELP: &str = r#"
Selecting fixtures

  fixture 3               select one, by its place in the patch
  fixture 1 thru 5        select a range
  fixture 1 thru 5 + 8    ranges combine with + and -
  fixture - 2             take one out of the current selection
  @ 80  /  at 80          the selection to 80% intensity
  full  /  out            the selection to 100% / 0%
  clear                   empty the programmer (locked values stay)
  clear clear             also drop the selection

What you set lives in the programmer until you store it:

  store sequence 2 cue 3  programmer into cue 3 of sequence 2
"#;

fn overview(catalog: &Catalog) -> String {
    let mut out = String::from(SELECTION_HELP.trim());
    out.push_str("\n\nWorking on the show\n\n");
    out.push_str("  create sequence \"Chases\"     add to a collection\n");
    out.push_str("  delete fixture 3             remove an entry\n");
    out.push_str("  rename sequence 2 \"Songs\"    change a name\n");
    out.push_str("  set cue 3 fade_time 4        change one field\n");
    out.push_str("\nCollections: ");
    out.push_str(&catalog.entity_words().join(", "));
    out.push('\n');

    let with_commands: Vec<&str> = {
        let mut tables: Vec<&str> =
            catalog.commands.iter().map(|c| c.table.as_str()).collect();
        tables.sort();
        tables.dedup();
        tables
    };
    if !with_commands.is_empty() {
        out.push_str("\nSome entries answer commands — `help <collection>` lists them:\n");
        for table in with_commands {
            out.push_str(&format!("  {} {} …\n", singular_of(table), example_of(catalog, table)));
        }
    }

    for prefix in catalog.rpc_prefixes() {
        out.push_str(&format!("\n`help {prefix}` — station commands ({prefix}.*)\n"));
    }
    out.trim_end().to_string()
}

fn example_of(catalog: &Catalog, table: &str) -> String {
    catalog
        .commands_of(table)
        .next()
        .map(|c| c.name.clone())
        .unwrap_or_default()
}

fn entity_help(catalog: &Catalog, table: &str) -> String {
    let word = singular_of(table);
    let mut out = format!("{word}\n\n");
    out.push_str(&format!("  {word} 2 <command>     by number, 1 is first\n"));
    out.push_str(&format!("  {word} \"Name\" …        or by name, in quotes\n"));

    let commands: Vec<_> = catalog.commands_of(table).collect();
    if commands.is_empty() {
        out.push_str("\nNo commands of its own. It can still be created, deleted,\nrenamed, and `set <field> <value>`.\n");
    } else {
        out.push_str("\nCommands:\n");
        for c in &commands {
            let args: Vec<String> = c
                .args
                .iter()
                .map(|a| {
                    if a.optional {
                        format!("[{}]", a.name)
                    } else {
                        format!("<{}>", a.name)
                    }
                })
                .collect();
            out.push_str(&format!("  {} {}\n", c.name, args.join(" ")));
            if !c.doc.is_empty() {
                for line in c.doc.lines() {
                    out.push_str(&format!("      {line}\n"));
                }
            }
        }
        if commands.iter().any(|c| c.name == "goNext") {
            out.push_str("\n  `go` is goNext; `goto <cue>` is goToCue by cue number.\n");
        }
    }

    if let Some(entity) = catalog.entities.iter().find(|e| e.table == table) {
        out.push_str("\nFields for `set`:\n  ");
        out.push_str(&entity.fields.join(", "));
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn rpc_help(catalog: &Catalog, prefix: &str) -> String {
    let mut out = format!("{prefix} commands\n\n");
    for rpc in &catalog.rpcs {
        let Some(method) = rpc.method.strip_prefix(&format!("{prefix}.")) else { continue };
        let args: Vec<String> = rpc.args.iter().map(|a| format!("<{}>", a.name)).collect();
        out.push_str(&format!("  {prefix} {method} {}\n", args.join(" ")));
        if !rpc.doc.is_empty() {
            out.push_str(&format!("      {}\n", rpc.doc));
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::test_catalog;

    #[test]
    fn the_overview_teaches_the_opening_moves() {
        let text = help(&test_catalog(), None);
        assert!(text.contains("fixture 1 thru 5"));
        assert!(text.contains("create sequence"));
        assert!(text.contains("sequence"), "collections are listed");
    }

    #[test]
    fn entity_help_lists_commands_with_their_arguments_and_docs() {
        let text = help(&test_catalog(), Some("sequence"));
        assert!(text.contains("goNext [at]"), "{text}");
        assert!(text.contains("goToCue <cueId> [at]"), "{text}");
        assert!(text.contains("Take the next cue"), "the doc comment rode along");
        assert!(text.contains("`go` is goNext"));
    }

    #[test]
    fn an_unknown_topic_says_so_gently() {
        let text = help(&test_catalog(), Some("banana"));
        assert!(text.contains("Nothing called"));
    }
}
