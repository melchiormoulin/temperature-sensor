use std::{net::SocketAddr, path::PathBuf, time::Duration};

use clap::Parser;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    /// Address used by the health and optional Prometheus HTTP server.
    #[arg(long, default_value = "0.0.0.0:9100")]
    pub listen: SocketAddr,

    /// Linux 1-Wire device directory containing 28-* sensors.
    #[arg(long, default_value = "/sys/bus/w1/devices")]
    pub device_root: PathBuf,

    /// Delay between sensor polling cycles.
    #[arg(long, default_value = "5s", value_parser = parse_nonzero_duration)]
    pub poll_interval: Duration,
}

fn parse_nonzero_duration(value: &str) -> Result<Duration, String> {
    let duration = humantime::parse_duration(value).map_err(|error| error.to_string())?;
    if duration.is_zero() {
        return Err("duration must be greater than zero".to_owned());
    }
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf, time::Duration};

    use clap::Parser;

    use super::Cli;

    #[test]
    fn uses_container_friendly_defaults() {
        let cli = Cli::try_parse_from(["temperature-sensor"]).expect("default CLI should parse");

        assert_eq!(cli.listen, SocketAddr::from(([0, 0, 0, 0], 9100)));
        assert_eq!(cli.device_root, PathBuf::from("/sys/bus/w1/devices"));
        assert_eq!(cli.poll_interval, Duration::from_secs(5));
    }

    #[test]
    fn rejects_zero_poll_interval() {
        let error = Cli::try_parse_from(["temperature-sensor", "--poll-interval", "0s"])
            .expect_err("zero interval should be rejected");

        assert!(error.to_string().contains("greater than zero"));
    }
}
