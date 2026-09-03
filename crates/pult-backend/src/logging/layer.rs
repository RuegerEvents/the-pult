//! A `tracing` event, turned into something a panel can show.

use pult_schema::ws::{LogLevel, LogSource};
use tracing::{field::Field, Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use super::LogHandle;

/// Keeps every event the subscriber's filter let through and this station's
/// capture level wants.
///
/// Sits beside the `fmt` layer rather than replacing it, so stdout is unchanged and
/// a console started from a shell behaves exactly as it did.
pub struct CaptureLayer {
    handle: LogHandle,
}

impl CaptureLayer {
    pub fn new(handle: LogHandle) -> CaptureLayer {
        CaptureLayer { handle }
    }
}

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = level_of(*event.metadata().level());
        // Checked before the visitor runs, so a `debug!` nobody is capturing costs
        // a compare rather than a formatted string.
        if !level.passes(self.handle.capture_level()) {
            return;
        }

        let mut visitor = Visitor::default();
        event.record(&mut visitor);

        let source = match visitor.plugin.take() {
            Some(id) => LogSource::Plugin(id),
            None => LogSource::Station,
        };
        self.handle.emit(level, event.metadata().target(), source, visitor.finish());
    }
}

fn level_of(level: Level) -> LogLevel {
    match level {
        Level::ERROR => LogLevel::Error,
        Level::WARN => LogLevel::Warn,
        Level::INFO => LogLevel::Info,
        Level::DEBUG => LogLevel::Debug,
        Level::TRACE => LogLevel::Trace,
    }
}

/// Pulls the message out of an event, and the plugin id out from beside it.
///
/// `plugin` is lifted into [`LogSource`] rather than left in the text: the panel
/// filters on the field, which a message containing a bracket cannot defeat. Any
/// other field is appended as `name=value`, which is what the `fmt` layer does and
/// so is what a reader of `.demo/backend.log` already expects.
#[derive(Default)]
struct Visitor {
    message: String,
    fields: String,
    plugin: Option<String>,
}

impl Visitor {
    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else {
            format!("{}{}", self.message, self.fields)
        }
    }

    fn field(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => self.message = value,
            "plugin" => self.plugin = Some(value),
            name => {
                self.fields.push(' ');
                self.fields.push_str(name);
                self.fields.push('=');
                self.fields.push_str(&value);
            }
        }
    }
}

impl tracing::field::Visit for Visitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.field(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.field(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.field(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.field(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.field(field, value.to_string());
    }
}
