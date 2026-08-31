//! The command line as maths.
//!
//! Everything here is pure: what the words mean, what could come next, what an
//! error should point at. The plugin around it does the talking — reads the
//! show to resolve names, writes the programmer, calls commands.
//!
//! The vocabulary is not written down anywhere in this crate: tables, fields,
//! commands and station RPCs all arrive in the [`Catalog`], which the plugin
//! fills from the console's introspection at startup. A new entity type in the
//! schema appears in the grammar with no change here.

mod catalog;
mod complete;
mod help;
mod ids;
mod parse;
mod token;

pub use catalog::{ArgInfo, Catalog, CommandInfo, EntityInfo, RpcInfo};
pub use complete::{complete, Completions, Expectation};
pub use help::help;
pub use ids::entry_id;
pub use parse::parse;
pub use token::{tokenize, Token, TokenKind};

/// Where something sits in the input, in byte offsets.
pub type Span = (usize, usize);

/// A command the operator asked for, resolved as far as words alone allow.
/// Names and 1-based numbers stay symbolic — turning them into uuids takes the
/// show, which is the executor's job.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `fixture 1 thru 5 + 7 - 2` — change the selection. With `at`, the
    /// combined form every console manual opens with: `fixture 1 thru 5 @ 80`
    /// selects *and* sets intensity on what it selected, in one line.
    Select { ops: Vec<(SelOp, Range)>, at: Option<f64> },
    /// `clear` empties the programmer; `clear clear` also drops the selection.
    Clear { also_selection: bool },
    /// `at 80`, `full`, `out` — intensity on the current selection, in percent.
    Intensity { percent: f64 },
    /// `sequence 2 go` — a registered entity command.
    EntityCommand {
        table: String,
        target: Target,
        /// The registered camelCase name, already resolved from what was typed.
        command: String,
        /// Positional values, paired with arg names from the schema.
        args: Vec<(String, serde_json::Value)>,
    },
    /// `create sequence "Chases"`.
    Create { table: String, name: Option<String> },
    /// `delete fixture 3`.
    Delete { table: String, target: Target },
    /// `set sequence 2 name "Songs"` — `rename` is sugar for the name field.
    SetField {
        table: String,
        target: Target,
        field: String,
        value: serde_json::Value,
    },
    /// `store sequence 2 cue 3` — programmer into a cue.
    Store { sequence: Target, cue: Target },
    /// `session join <id>`, `device adopt <serial>` — a station RPC.
    Rpc {
        method: String,
        args: Vec<(String, serde_json::Value)>,
    },
    /// `help [topic]`.
    Help { topic: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// 1-based position in the collection's display order.
    Index(usize),
    /// A quoted name, matched against the entity's `name` field.
    Name(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelOp {
    Replace,
    Add,
    Remove,
}

/// A 1-based inclusive range; a single number is `n thru n`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub from: usize,
    pub to: usize,
}

/// What went wrong, where, and what would have been accepted.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
    pub expected: Vec<String>,
}

impl ParseError {
    pub(crate) fn new(message: impl Into<String>, span: Span) -> Self {
        Self { message: message.into(), span, expected: Vec::new() }
    }

    pub(crate) fn expecting(mut self, expected: Vec<String>) -> Self {
        self.expected = expected;
        self
    }
}
