mod shared;

use opentelemetry::{
    global,
    propagation::TextMapPropagator,
    trace::{Status, TraceContextExt, Tracer, TracerProvider},
    Context, KeyValue,
};
use opentelemetry_sdk::trace::SdkTracerProvider;
use sentry_core::protocol::Transaction;
use sentry_opentelemetry::{SentryPropagator, SentrySpanProcessor};
use std::collections::HashMap;

#[test]
fn test_creates_distributed_trace() {
    println!("init transport");
    let transport = shared::init_sentry(1.0); // Sample all spans

    // Set up OpenTelemetry
    println!("set propagator");
    global::set_text_map_propagator(SentryPropagator::new());
    println!("tracer provider");
    let tracer_provider = SdkTracerProvider::builder()
        .with_span_processor(SentrySpanProcessor::new())
        .build();
    println!("tracer");
    let tracer = tracer_provider.tracer("test".to_string());

    // Create a "first service" span
    println!("first_service");
    let first_service_span = tracer.start("first_service");
    let first_service_ctx = Context::current_with_span(first_service_span);

    // Simulate passing the context to another service by extracting and injecting e.g. HTTP
    // headers
    println!("propagator");
    let propagator = SentryPropagator::new();
    let mut headers = HashMap::new();
    propagator.inject_context(&first_service_ctx, &mut TestInjector(&mut headers));

    // End the first service span
    println!("end first service");
    first_service_ctx.span().end();

    // Check that the first service sent data to Sentry
    println!("fetch and clear envelopes");
    let first_envelopes = transport.fetch_and_clear_envelopes();
    assert_eq!(first_envelopes.len(), 1);

    println!("first tx");
    let first_tx = match first_envelopes[0].items().next().unwrap() {
        sentry::protocol::EnvelopeItem::Transaction(tx) => tx.clone(),
        _ => panic!("Expected transaction"),
    };

    // Get first service trace ID and span ID
    println!("first trace id and span id");
    let (first_trace_id, first_span_id) = match &first_tx.contexts.get("trace") {
        Some(sentry::protocol::Context::Trace(trace)) => (trace.trace_id, trace.span_id),
        _ => panic!("Missing trace context in first transaction"),
    };

    // Now simulate the second service receiving the headers and continuing the trace
    println!("extract with context");
    let second_service_ctx =
        propagator.extract_with_context(&Context::current(), &TestExtractor(&headers));

    // Create a second service span that continues the trace
    println!("second service span");
    let second_service_span = tracer.start_with_context("second_service", &second_service_ctx);
    let second_service_ctx = second_service_ctx.with_span(second_service_span);

    // End the second service span
    println!("end second service");
    second_service_ctx.span().end();

    // Check that the second service sent data to Sentry
    println!("fetch and clear envelopes 2");
    let second_envelopes = transport.fetch_and_clear_envelopes();
    assert_eq!(second_envelopes.len(), 1);

    println!("second tx");
    let second_tx = match second_envelopes[0].items().next().unwrap() {
        sentry::protocol::EnvelopeItem::Transaction(tx) => tx.clone(),
        _ => panic!("Expected transaction"),
    };

    // Get second service trace ID and span ID
    println!("second trace id and span id");
    let (second_trace_id, second_span_id, second_parent_span_id) =
        match &second_tx.contexts.get("trace") {
            Some(sentry::protocol::Context::Trace(trace)) => {
                (trace.trace_id, trace.span_id, trace.parent_span_id)
            }
            _ => panic!("Missing trace context in second transaction"),
        };

    // Verify the distributed trace - same trace ID, different span IDs
    println!("asserts");
    assert_eq!(first_trace_id, second_trace_id, "Trace IDs should match");
    assert_ne!(
        first_span_id, second_span_id,
        "Span IDs should be different"
    );
    assert_eq!(
        second_parent_span_id,
        Some(first_span_id),
        "Second service's parent span ID should match first service's span ID"
    );
    println!("test logic done");

    let x = tracer_provider.shutdown();
    println!("{:?}", x);
    println!("shut down provider");
}

struct TestInjector<'a>(&'a mut HashMap<String, String>);

impl opentelemetry::propagation::Injector for TestInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
}

struct TestExtractor<'a>(&'a HashMap<String, String>);

impl opentelemetry::propagation::Extractor for TestExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}
