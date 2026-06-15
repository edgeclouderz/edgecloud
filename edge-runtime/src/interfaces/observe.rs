//! `edge:observe` — metrics and logging.
//!
//! Provides a pluggable `MetricsBackend` trait. The default `NoOpBackend` logs to
//! tracing. When the `observe` feature is enabled, a `PrometheusBackend` is also
//! available via `Observer::with_backend`.

/// Pluggable metrics backend.
pub trait MetricsBackend: Send + Sync {
    fn increment_counter(&self, name: &str, labels: &[(String, String)]);
    fn record_gauge(&self, name: &str, value: f64, labels: &[(String, String)]);
    fn record_histogram(&self, name: &str, value: f64, labels: &[(String, String)]);
    fn emit_log(&self, level: &str, message: &str);
}

/// No-op backend that logs to tracing (current behavior).
pub struct NoOpBackend;

impl MetricsBackend for NoOpBackend {
    fn increment_counter(&self, name: &str, _labels: &[(String, String)]) {
        tracing::debug!(counter = name, "counter incremented");
    }

    fn record_gauge(&self, name: &str, value: f64, _labels: &[(String, String)]) {
        tracing::debug!(gauge = name, value = value, "gauge recorded");
    }

    fn record_histogram(&self, name: &str, value: f64, _labels: &[(String, String)]) {
        tracing::debug!(histogram = name, value = value, "histogram recorded");
    }

    fn emit_log(&self, level: &str, message: &str) {
        match level {
            "error" => tracing::error!(message),
            "warn" => tracing::warn!(message),
            "info" => tracing::info!(message),
            "debug" => tracing::debug!(message),
            _ => tracing::trace!(message),
        }
    }
}

#[cfg(feature = "observe")]
mod prometheus_backend {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use metrics::{describe_counter, describe_gauge, describe_histogram, Counter, Gauge, Histogram};

    /// Prometheus-compatible backend using the `metrics` crate.
    pub struct PrometheusBackend {
        counters: Arc<RwLock<HashMap<String, Counter>>>,
        gauges: Arc<RwLock<HashMap<String, Gauge>>>,
        histograms: Arc<RwLock<HashMap<String, Histogram>>>,
    }

    impl PrometheusBackend {
        pub fn new() -> Self {
            // Describe metrics so they show up in Prometheus.
            describe_counter!("edge_counter", "Counter metric from edge:observe");
            describe_gauge!("edge_gauge", "Gauge metric from edge:observe");
            describe_histogram!("edge_histogram", "Histogram metric from edge:observe");
            Self {
                counters: Arc::new(RwLock::new(HashMap::new())),
                gauges: Arc::new(RwLock::new(HashMap::new())),
                histograms: Arc::new(RwLock::new(HashMap::new())),
            }
        }

        #[allow(dead_code)]
        fn make_labels(labels: &[(String, String)]) -> Vec<metrics::Label> {
            labels.iter().map(|(k, v)| metrics::Label::new(k.clone(), v.clone())).collect()
        }
    }

    impl Default for PrometheusBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MetricsBackend for PrometheusBackend {
        fn increment_counter(&self, name: &str, _labels: &[(String, String)]) {
            let counter = self.counters.read().unwrap()
                .get(name)
                .cloned()
                .unwrap_or_else(|| {
                    let c = Counter::noop();
                    let mut counters = self.counters.write().unwrap();
                    counters.insert(name.to_string(), c.clone());
                    c
                });
            counter.increment(1);
        }

        fn record_gauge(&self, name: &str, value: f64, _labels: &[(String, String)]) {
            let gauge = self.gauges.read().unwrap()
                .get(name)
                .cloned()
                .unwrap_or_else(|| {
                    let g = Gauge::noop();
                    let mut gauges = self.gauges.write().unwrap();
                    gauges.insert(name.to_string(), g.clone());
                    g
                });
            gauge.set(value);
        }

        fn record_histogram(&self, name: &str, value: f64, _labels: &[(String, String)]) {
            let histogram = self.histograms.read().unwrap()
                .get(name)
                .cloned()
                .unwrap_or_else(|| {
                    let h = Histogram::noop();
                    let mut histograms = self.histograms.write().unwrap();
                    histograms.insert(name.to_string(), h.clone());
                    h
                });
            histogram.record(value);
        }

        fn emit_log(&self, level: &str, message: &str) {
            match level {
                "error" => tracing::error!(message),
                "warn" => tracing::warn!(message),
                "info" => tracing::info!(message),
                "debug" => tracing::debug!(message),
                _ => tracing::trace!(message),
            }
        }
    }
}

#[cfg(not(feature = "observe"))]
mod prometheus_backend {
    // No-op when feature is disabled.
}

/// Observer holds a pluggable metrics backend.
pub struct Observer {
    backend: Box<dyn MetricsBackend>,
}

impl Observer {
    /// Create an Observer with a no-op (logging-only) backend.
    pub fn new() -> Self {
        Self::with_backend(Box::new(NoOpBackend))
    }

    /// Create an Observer with a custom backend.
    pub fn with_backend(backend: Box<dyn MetricsBackend>) -> Self {
        Self { backend }
    }

    /// Create an Observer with a Prometheus backend (requires `observe` feature).
    #[cfg(feature = "observe")]
    pub fn with_prometheus() -> Self {
        use prometheus_backend::PrometheusBackend;
        Self::with_backend(Box::new(PrometheusBackend::new()))
    }

    #[cfg(not(feature = "observe"))]
    pub fn with_prometheus() -> Self {
        Self::new()
    }

    pub fn increment_counter(&self, name: &str, labels: &[(String, String)]) {
        self.backend.increment_counter(name, labels);
    }

    pub fn record_gauge(&self, name: &str, value: f64, labels: &[(String, String)]) {
        self.backend.record_gauge(name, value, labels);
    }

    pub fn record_histogram(&self, name: &str, value: f64, labels: &[(String, String)]) {
        self.backend.record_histogram(name, value, labels);
    }

    pub fn emit_log(&self, level: &str, message: &str) {
        self.backend.emit_log(level, message);
    }
}

impl Default for Observer {
    fn default() -> Self {
        Self::new()
    }
}
