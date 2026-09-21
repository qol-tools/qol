use tracing::Subscriber;
use tracing_subscriber::layer::Filter;
use tracing_subscriber::Layer;

pub(crate) struct ErrorCaptureLayer;

impl<S: Subscriber> Layer<S> for ErrorCaptureLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        let file = visitor
            .log_file
            .as_deref()
            .or(event.metadata().file())
            .unwrap_or("unknown");
        let line = visitor.log_line.or(event.metadata().line()).unwrap_or(0);
        let target = event.metadata().target();

        super::file_logger::on_error_event(target, &visitor.message, file, line);
    }
}

pub(crate) struct ErrorOnlyFilter;

impl<S: Subscriber> Filter<S> for ErrorOnlyFilter {
    fn enabled(
        &self,
        metadata: &tracing::Metadata<'_>,
        _cx: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        *metadata.level() <= tracing::Level::ERROR
    }
}

#[derive(Default)]
struct EventVisitor {
    message: String,
    log_file: Option<String>,
    log_line: Option<u32>,
}

impl tracing::field::Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_string(),
            "log.file" => self.log_file = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if field.name() == "log.line" {
            self.log_line = Some(value as u32);
        }
    }
}
