use tracing::Subscriber;
use tracing_subscriber::Layer;

pub(crate) struct ErrorCaptureLayer;

impl<S: Subscriber> Layer<S> for ErrorCaptureLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if *event.metadata().level() != tracing::Level::ERROR {
            return;
        }

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let file = event.metadata().file().unwrap_or("unknown");
        let line = event.metadata().line().unwrap_or(0);
        let target = event.metadata().target();

        super::prod::on_error_event(target, &visitor.message, file, line);
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}
