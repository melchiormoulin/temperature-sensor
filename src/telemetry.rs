use std::{env, sync::Arc};

use anyhow::{Context, Result};
use opentelemetry::{InstrumentationScope, KeyValue, metrics::MeterProvider as _};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider},
};
use prometheus::Registry;
use tracing::warn;

use crate::{
    generated::{attributes, metrics::Instruments, schema::SCHEMA_URL},
    sensor::{SensorFailure, SharedSnapshot, snapshot},
};

const OTEL_METRICS_EXPORTER: &str = "OTEL_METRICS_EXPORTER";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetricsExporter {
    Otlp,
    Prometheus,
    None,
}

impl MetricsExporter {
    fn from_env() -> Self {
        let configured = env::var(OTEL_METRICS_EXPORTER).ok();
        Self::parse(configured.as_deref())
    }

    fn parse(configured: Option<&str>) -> Self {
        let configured = configured
            .filter(|value| !value.is_empty())
            .unwrap_or("otlp");

        if configured.eq_ignore_ascii_case("otlp") {
            Self::Otlp
        } else if configured.eq_ignore_ascii_case("prometheus") {
            Self::Prometheus
        } else if configured.eq_ignore_ascii_case("none") {
            Self::None
        } else {
            warn!(
                value = configured,
                "unsupported OTEL_METRICS_EXPORTER value; falling back to otlp"
            );
            Self::Otlp
        }
    }
}

pub struct Telemetry {
    provider: SdkMeterProvider,
    registry: Option<Registry>,
    instruments: Instruments,
}

impl Telemetry {
    pub fn initialize(state: SharedSnapshot) -> Result<Self> {
        match MetricsExporter::from_env() {
            MetricsExporter::Otlp => {
                let exporter = opentelemetry_otlp::MetricExporter::builder()
                    .with_http()
                    .with_protocol(Protocol::HttpBinary)
                    .build()
                    .context("failed to configure the OTLP HTTP/protobuf exporter")?;
                Ok(Self::with_otlp_exporter(state, exporter))
            }
            MetricsExporter::Prometheus => Self::with_prometheus_exporter(state),
            MetricsExporter::None => Ok(Self::build(state, None, provider_builder())),
        }
    }

    fn with_otlp_exporter(
        state: SharedSnapshot,
        otlp_exporter: opentelemetry_otlp::MetricExporter,
    ) -> Self {
        let otlp_reader = PeriodicReader::builder(otlp_exporter).build();

        Self::build(state, None, provider_builder().with_reader(otlp_reader))
    }

    fn with_prometheus_exporter(state: SharedSnapshot) -> Result<Self> {
        let (registry, provider_builder) = prometheus_provider_builder()?;
        Ok(Self::build(state, Some(registry), provider_builder))
    }

    #[must_use]
    pub fn registry(&self) -> Option<Registry> {
        self.registry.clone()
    }

    pub fn record_sensor_failure(&self, failure: &SensorFailure) {
        self.instruments.hw_errors.add(
            1,
            &[
                KeyValue::new(attributes::ERROR_TYPE, failure.error_type),
                KeyValue::new(attributes::HW_ID, failure.sensor_id.clone()),
                KeyValue::new(attributes::HW_TYPE, attributes::HW_TYPE_TEMPERATURE),
            ],
        );
    }

    pub async fn shutdown(&self) -> Result<()> {
        let provider = self.provider.clone();
        tokio::task::spawn_blocking(move || provider.shutdown())
            .await
            .context("OpenTelemetry shutdown task failed")?
            .context("failed to flush and shut down OpenTelemetry metrics")
    }

    fn build(
        state: SharedSnapshot,
        registry: Option<Registry>,
        provider_builder: MeterProviderBuilder,
    ) -> Self {
        let provider = provider_builder.build();
        let scope = InstrumentationScope::builder(env!("CARGO_PKG_NAME"))
            .with_version(env!("CARGO_PKG_VERSION"))
            .with_schema_url(SCHEMA_URL)
            .build();
        let meter = provider.meter_with_scope(scope);

        let count_state = Arc::clone(&state);
        let up_state = Arc::clone(&state);
        let temperature_state = state;
        let instruments = Instruments::new(
            &meter,
            move |observer| {
                let current = snapshot(&count_state);
                observer.observe(u64::try_from(current.discovered).unwrap_or(u64::MAX), &[]);
            },
            move |observer| {
                let current = snapshot(&up_state);
                for (sensor_id, reading) in current.sensors {
                    observer.observe(
                        u64::from(reading.up),
                        &[KeyValue::new(attributes::HW_ID, sensor_id)],
                    );
                }
            },
            move |observer| {
                let current = snapshot(&temperature_state);
                for (sensor_id, reading) in current.sensors {
                    if let Some(temperature_celsius) = reading.temperature_celsius {
                        observer.observe(
                            temperature_celsius,
                            &[KeyValue::new(attributes::HW_ID, sensor_id)],
                        );
                    }
                }
            },
        );

        Self {
            provider,
            registry,
            instruments,
        }
    }

    #[cfg(test)]
    fn prometheus_only(state: SharedSnapshot) -> Result<Self> {
        Self::with_prometheus_exporter(state)
    }
}

fn prometheus_provider_builder() -> Result<(Registry, MeterProviderBuilder)> {
    let registry = Registry::new();
    let prometheus_reader = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_scope_info()
        .build()
        .context("failed to configure the Prometheus exporter")?;
    let provider_builder = provider_builder().with_reader(prometheus_reader);

    Ok((registry, provider_builder))
}

fn provider_builder() -> MeterProviderBuilder {
    let service_name = env::var("OTEL_SERVICE_NAME")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_NAME").to_owned());
    let resource = Resource::builder()
        .with_service_name(service_name)
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .build();

    SdkMeterProvider::builder().with_resource(resource)
}

#[cfg(test)]
mod tests {
    use std::{future::IntoFuture, sync::PoisonError, time::Duration};

    use axum::{
        Router,
        body::Bytes,
        extract::State,
        http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
        routing::post,
    };
    use opentelemetry_otlp::{Protocol, WithExportConfig};
    use prometheus::{Encoder, TextEncoder};
    use tokio::sync::{mpsc, oneshot};

    use super::{MetricsExporter, Telemetry};
    use crate::sensor::{SensorFailure, SensorReading, new_shared_snapshot};

    #[test]
    fn parses_official_metrics_exporter_values() {
        assert_eq!(MetricsExporter::parse(None), MetricsExporter::Otlp);
        assert_eq!(MetricsExporter::parse(Some("")), MetricsExporter::Otlp);
        assert_eq!(MetricsExporter::parse(Some("OTLP")), MetricsExporter::Otlp);
        assert_eq!(
            MetricsExporter::parse(Some("prometheus")),
            MetricsExporter::Prometheus
        );
        assert_eq!(MetricsExporter::parse(Some("none")), MetricsExporter::None);
        assert_eq!(
            MetricsExporter::parse(Some("unsupported")),
            MetricsExporter::Otlp
        );
    }

    #[test]
    fn exports_generated_metrics_with_semantic_convention_attributes() {
        let state = new_shared_snapshot();
        {
            let mut snapshot = state.write().unwrap_or_else(PoisonError::into_inner);
            snapshot.discovered = 1;
            snapshot.sensors.insert(
                "28-000000000001".to_owned(),
                SensorReading {
                    temperature_celsius: Some(24.562),
                    up: true,
                },
            );
        }
        let telemetry = Telemetry::prometheus_only(state).expect("telemetry should initialize");
        telemetry.record_sensor_failure(&SensorFailure {
            sensor_id: "28-000000000001".to_owned(),
            error_type: "crc",
            message: "test error".to_owned(),
        });

        let registry = telemetry
            .registry()
            .expect("Prometheus registry should be configured");
        let families = registry.gather();
        let mut output = Vec::new();
        TextEncoder::new()
            .encode(&families, &mut output)
            .expect("metrics should encode");
        let output = String::from_utf8(output).expect("Prometheus output should be UTF-8");

        assert!(
            output.contains("hw_temperature_celsius{hw_id=\"28-000000000001\"} 24.562"),
            "{output}"
        );
        assert!(output.contains("fishtank_sensor_up_ratio{hw_id=\"28-000000000001\"} 1"));
        assert!(output.contains("fishtank_sensor_count 1"));
        assert!(output.contains("error_type=\"crc\""));
        assert!(output.contains("hw_type=\"temperature\""));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sends_temperature_over_otlp_http_protobuf() {
        let state = new_shared_snapshot();
        {
            let mut snapshot = state.write().unwrap_or_else(PoisonError::into_inner);
            snapshot.discovered = 1;
            snapshot.sensors.insert(
                "28-000000000001".to_owned(),
                SensorReading {
                    temperature_celsius: Some(24.562),
                    up: true,
                },
            );
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock OTLP listener");
        let address = listener.local_addr().expect("mock listener address");
        let (request_tx, mut request_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let router = Router::new()
            .route("/v1/metrics", post(capture_otlp_request))
            .with_state(request_tx);
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .into_future()
                .await
        });

        let exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(format!("http://{address}/v1/metrics"))
            .with_timeout(Duration::from_secs(2))
            .build()
            .expect("test OTLP exporter");
        let telemetry = Telemetry::with_otlp_exporter(state, exporter);
        let provider = telemetry.provider.clone();
        tokio::task::spawn_blocking(move || provider.force_flush())
            .await
            .expect("flush task")
            .expect("OTLP flush");

        let (headers, body) = tokio::time::timeout(Duration::from_secs(2), request_rx.recv())
            .await
            .expect("OTLP request timeout")
            .expect("OTLP request");
        assert_eq!(headers[CONTENT_TYPE], "application/x-protobuf");
        assert!(
            body.windows(b"hw.temperature".len())
                .any(|window| window == b"hw.temperature"),
            "OTLP payload did not contain the generated temperature metric"
        );

        telemetry.shutdown().await.expect("telemetry shutdown");
        let _ = shutdown_tx.send(());
        server
            .await
            .expect("mock server task")
            .expect("mock server");
    }

    async fn capture_otlp_request(
        State(sender): State<mpsc::Sender<(HeaderMap, Bytes)>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        sender
            .send((headers, body))
            .await
            .map_or(StatusCode::INTERNAL_SERVER_ERROR, |()| StatusCode::OK)
    }
}
