use std::{future::IntoFuture, io, sync::Arc};

use anyhow::{Context, Result};
use tokio::{
    sync::watch,
    time::{MissedTickBehavior, interval},
};
use tracing::{info, warn};

use crate::{
    cli::Cli,
    http,
    sensor::{PollReport, SharedSnapshot, ensure_device_root, new_shared_snapshot, poll_once},
    telemetry::Telemetry,
};

pub async fn run(cli: Cli) -> Result<()> {
    ensure_device_root(&cli.device_root)
        .await
        .with_context(|| {
            format!(
                "1-Wire device root {} is not readable",
                cli.device_root.display()
            )
        })?;

    let state = new_shared_snapshot();
    let telemetry = Arc::new(Telemetry::initialize(Arc::clone(&state))?);
    let initial_report = poll_once(&cli.device_root, &state)
        .await
        .context("initial sensor discovery failed")?;
    process_report(&initial_report, &telemetry);

    let listener = tokio::net::TcpListener::bind(cli.listen)
        .await
        .with_context(|| format!("failed to bind HTTP server to {}", cli.listen))?;
    let local_address = listener
        .local_addr()
        .context("failed to inspect HTTP listener")?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let poll_task = tokio::spawn(polling_loop(
        cli.device_root,
        cli.poll_interval,
        Arc::clone(&state),
        Arc::clone(&telemetry),
        shutdown_rx.clone(),
    ));
    let server = axum::serve(listener, http::router(telemetry.registry()))
        .with_graceful_shutdown(wait_for_shutdown(shutdown_rx))
        .into_future();
    tokio::pin!(server);

    info!(address = %local_address, "temperature sensor service started");
    let server_result = tokio::select! {
        signal_result = shutdown_signal() => {
            match signal_result {
                Ok(()) => {
                    info!("shutdown signal received");
                    let _ = shutdown_tx.send(true);
                    server.await
                }
                Err(error) => Err(error),
            }
        }
        result = &mut server => result,
    };

    let _ = shutdown_tx.send(true);
    poll_task.await.context("sensor polling task failed")?;
    telemetry.shutdown().await?;
    server_result.context("HTTP server failed")
}

async fn polling_loop(
    device_root: std::path::PathBuf,
    poll_interval: std::time::Duration,
    state: SharedSnapshot,
    telemetry: Arc<Telemetry>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ticker = interval(poll_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                tokio::select! {
                    result = poll_once(&device_root, &state) => {
                        match result {
                            Ok(report) => process_report(&report, &telemetry),
                            Err(error) => {
                                process_report(&error.report, &telemetry);
                                warn!(error = %error, "sensor discovery failed");
                            }
                        }
                    }
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

fn process_report(report: &PollReport, telemetry: &Telemetry) {
    for failure in &report.failures {
        telemetry.record_sensor_failure(failure);
    }

    for transition in &report.transitions {
        if transition.up {
            info!(sensor.id = %transition.sensor_id, "temperature sensor is available");
        } else {
            let failure = report
                .failures
                .iter()
                .find(|failure| failure.sensor_id == transition.sensor_id);
            warn!(
                sensor.id = %transition.sensor_id,
                error.type = failure.map_or("unknown", |failure| failure.error_type),
                error.message = failure.map_or("sensor read failed", |failure| failure.message.as_str()),
                "temperature sensor is unavailable"
            );
        }
    }
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> io::Result<()> {
    tokio::signal::ctrl_c().await
}
