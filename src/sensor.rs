use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock},
    time::Duration,
};

use thiserror::Error;

const FAMILY_PREFIX: &str = "28-";
const MIN_TEMPERATURE_MILLIDEGREES: i32 = -55_000;
const MAX_TEMPERATURE_MILLIDEGREES: i32 = 125_000;
const READ_TIMEOUT: Duration = Duration::from_secs(2);

pub type SharedSnapshot = Arc<RwLock<SensorSnapshot>>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SensorSnapshot {
    pub discovered: usize,
    pub sensors: BTreeMap<String, SensorReading>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SensorReading {
    pub temperature_celsius: Option<f64>,
    pub up: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SensorFailure {
    pub sensor_id: String,
    pub error_type: &'static str,
    pub message: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SensorTransition {
    pub sensor_id: String,
    pub up: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PollReport {
    pub failures: Vec<SensorFailure>,
    pub transitions: Vec<SensorTransition>,
}

#[derive(Debug, Error)]
#[error("sensor discovery failed: {source}")]
pub struct PollError {
    #[source]
    source: io::Error,
    pub report: PollReport,
}

#[derive(Debug, Error)]
pub enum ReadError {
    #[error("failed to read sensor file: {0}")]
    Io(#[from] io::Error),
    #[error("sensor read timed out")]
    Timeout,
    #[error("sensor CRC check failed")]
    Crc,
    #[error("sensor file has an invalid format: {0}")]
    Format(&'static str),
    #[error("invalid temperature value: {0}")]
    InvalidTemperature(#[from] std::num::ParseIntError),
    #[error("temperature {0} millidegrees Celsius is outside the DS18B20 range")]
    OutOfRange(i32),
}

impl ReadError {
    #[must_use]
    pub const fn error_type(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Timeout => "timeout",
            Self::Crc => "crc",
            Self::Format(_) | Self::InvalidTemperature(_) => "format",
            Self::OutOfRange(_) => "range",
        }
    }
}

#[must_use]
pub fn new_shared_snapshot() -> SharedSnapshot {
    Arc::new(RwLock::new(SensorSnapshot::default()))
}

#[must_use]
pub fn snapshot(state: &SharedSnapshot) -> SensorSnapshot {
    state.read().unwrap_or_else(PoisonError::into_inner).clone()
}

pub async fn ensure_device_root(device_root: &Path) -> io::Result<()> {
    let metadata = tokio::fs::metadata(device_root).await?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a directory", device_root.display()),
        ))
    }
}

pub async fn poll_once(
    device_root: &Path,
    state: &SharedSnapshot,
) -> Result<PollReport, PollError> {
    let discovered = match discover(device_root).await {
        Ok(discovered) => discovered,
        Err(source) => {
            let report = mark_all_unavailable(state, &source);
            return Err(PollError { source, report });
        }
    };

    let previous = snapshot(state);
    let discovered_ids = discovered
        .iter()
        .map(|sensor| sensor.id.clone())
        .collect::<BTreeSet<_>>();
    let mut next = SensorSnapshot {
        discovered: discovered.len(),
        sensors: previous
            .sensors
            .iter()
            .filter(|(_, reading)| reading.up)
            .map(|(id, _)| (id.clone(), SensorReading::default()))
            .collect(),
    };
    let mut report = PollReport::default();

    for sensor in discovered {
        match read_temperature(&sensor.path).await {
            Ok(temperature_celsius) => {
                next.sensors.insert(
                    sensor.id.clone(),
                    SensorReading {
                        temperature_celsius: Some(temperature_celsius),
                        up: true,
                    },
                );
            }
            Err(error) => {
                report.failures.push(SensorFailure {
                    sensor_id: sensor.id.clone(),
                    error_type: error.error_type(),
                    message: error.to_string(),
                });
                next.sensors
                    .insert(sensor.id.clone(), SensorReading::default());
            }
        }
    }

    for (sensor_id, old_reading) in &previous.sensors {
        let new_up = next
            .sensors
            .get(sensor_id)
            .is_some_and(|reading| reading.up);
        if old_reading.up != new_up {
            report.transitions.push(SensorTransition {
                sensor_id: sensor_id.clone(),
                up: new_up,
            });
        }
        if old_reading.up && !discovered_ids.contains(sensor_id) {
            report.failures.push(SensorFailure {
                sensor_id: sensor_id.clone(),
                error_type: "not_found",
                message: "sensor disappeared from the 1-Wire device directory".to_owned(),
            });
        }
    }

    for (sensor_id, reading) in &next.sensors {
        if !previous.sensors.contains_key(sensor_id) {
            report.transitions.push(SensorTransition {
                sensor_id: sensor_id.clone(),
                up: reading.up,
            });
        }
    }

    *state.write().unwrap_or_else(PoisonError::into_inner) = next;
    Ok(report)
}

pub fn parse_temperature(contents: &str) -> Result<f64, ReadError> {
    let mut lines = contents.lines();
    let crc_line = lines.next().ok_or(ReadError::Format("missing CRC line"))?;
    let data_line = lines
        .next()
        .ok_or(ReadError::Format("missing temperature line"))?;

    match crc_line.split_ascii_whitespace().next_back() {
        Some("YES") => {}
        Some("NO") => return Err(ReadError::Crc),
        _ => return Err(ReadError::Format("missing CRC result")),
    }

    let raw_temperature = data_line
        .split_ascii_whitespace()
        .find_map(|field| field.strip_prefix("t="))
        .ok_or(ReadError::Format("missing t= field"))?
        .parse::<i32>()?;

    if !(MIN_TEMPERATURE_MILLIDEGREES..=MAX_TEMPERATURE_MILLIDEGREES).contains(&raw_temperature) {
        return Err(ReadError::OutOfRange(raw_temperature));
    }

    Ok(f64::from(raw_temperature) / 1000.0)
}

#[derive(Debug)]
struct DiscoveredSensor {
    id: String,
    path: PathBuf,
}

async fn discover(device_root: &Path) -> io::Result<Vec<DiscoveredSensor>> {
    let mut directory = tokio::fs::read_dir(device_root).await?;
    let mut sensors = Vec::new();

    while let Some(entry) = directory.next_entry().await? {
        let name = entry.file_name();
        let Some(id) = name.to_str() else {
            continue;
        };
        if id.starts_with(FAMILY_PREFIX) && id.len() > FAMILY_PREFIX.len() {
            sensors.push(DiscoveredSensor {
                id: id.to_owned(),
                path: entry.path().join("w1_slave"),
            });
        }
    }

    sensors.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    Ok(sensors)
}

async fn read_temperature(path: &Path) -> Result<f64, ReadError> {
    let contents = read_with_timeout(tokio::fs::read_to_string(path), READ_TIMEOUT).await?;
    parse_temperature(&contents)
}

async fn read_with_timeout(
    read: impl Future<Output = io::Result<String>>,
    timeout: Duration,
) -> Result<String, ReadError> {
    tokio::time::timeout(timeout, read)
        .await
        .map_err(|_| ReadError::Timeout)?
        .map_err(ReadError::Io)
}

fn mark_all_unavailable(state: &SharedSnapshot, error: &io::Error) -> PollReport {
    let previous = snapshot(state);
    let mut unavailable = SensorSnapshot::default();
    let mut report = PollReport::default();

    for (sensor_id, reading) in previous.sensors {
        if reading.up {
            unavailable
                .sensors
                .insert(sensor_id.clone(), SensorReading::default());
            report.failures.push(SensorFailure {
                sensor_id: sensor_id.clone(),
                error_type: "io",
                message: format!("sensor discovery failed: {error}"),
            });
            report.transitions.push(SensorTransition {
                sensor_id,
                up: false,
            });
        }
    }

    *state.write().unwrap_or_else(PoisonError::into_inner) = unavailable;
    report
}

#[cfg(test)]
mod tests {
    use std::{fs, future::pending, io, time::Duration};

    use tempfile::TempDir;

    use super::{
        ReadError, new_shared_snapshot, parse_temperature, poll_once, read_with_timeout, snapshot,
    };

    const VALID_READING: &str = "9e 01 4b 46 7f ff 02 10 56 : crc=56 YES\n\
                                 9e 01 4b 46 7f ff 02 10 56 t=25500\n";

    #[test]
    fn parses_positive_temperature() {
        assert_eq!(
            parse_temperature(VALID_READING).expect("valid reading"),
            25.5
        );
    }

    #[test]
    fn parses_negative_temperature() {
        let reading = "fc ff 4b 46 7f ff 04 10 8e : crc=8e YES\n\
                       fc ff 4b 46 7f ff 04 10 8e t=-250\n";

        assert_eq!(parse_temperature(reading).expect("valid reading"), -0.25);
    }

    #[test]
    fn rejects_failed_crc() {
        let reading = VALID_READING.replace("YES", "NO");

        assert!(matches!(parse_temperature(&reading), Err(ReadError::Crc)));
    }

    #[test]
    fn rejects_missing_temperature() {
        let reading = "9e 01 : crc=56 YES\n9e 01 no-temperature\n";

        assert!(matches!(
            parse_temperature(reading),
            Err(ReadError::Format("missing t= field"))
        ));
    }

    #[test]
    fn rejects_out_of_range_temperature() {
        let reading = "9e 01 : crc=56 YES\n9e 01 t=125001\n";

        assert!(matches!(
            parse_temperature(reading),
            Err(ReadError::OutOfRange(125_001))
        ));
    }

    #[tokio::test]
    async fn classifies_sensor_read_timeout() {
        let error = read_with_timeout(pending::<io::Result<String>>(), Duration::ZERO)
            .await
            .expect_err("pending read should time out");

        assert!(matches!(error, ReadError::Timeout));
        assert_eq!(error.error_type(), "timeout");
    }

    #[tokio::test]
    async fn discovers_only_ds18b20_devices() {
        let directory = TempDir::new().expect("temporary directory");
        write_sensor(directory.path(), "28-000000000002", VALID_READING);
        write_sensor(directory.path(), "28-000000000001", VALID_READING);
        write_sensor(directory.path(), "00-false-device", VALID_READING);
        fs::create_dir(directory.path().join("w1_bus_master1")).expect("bus directory");
        let state = new_shared_snapshot();

        let report = poll_once(directory.path(), &state)
            .await
            .expect("poll should succeed");
        let snapshot = snapshot(&state);

        assert!(report.failures.is_empty());
        assert_eq!(snapshot.discovered, 2);
        assert_eq!(
            snapshot.sensors.keys().cloned().collect::<Vec<_>>(),
            ["28-000000000001", "28-000000000002"]
        );
        assert!(snapshot.sensors.values().all(|reading| reading.up));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn follows_sysfs_style_sensor_symlink() {
        use std::os::unix::fs::symlink;

        let directory = TempDir::new().expect("temporary directory");
        let target = write_sensor(directory.path(), "device-target", VALID_READING);
        symlink(&target, directory.path().join("28-000000000001")).expect("sensor symlink");
        let state = new_shared_snapshot();

        poll_once(directory.path(), &state)
            .await
            .expect("poll should follow symlink");
        let snapshot = snapshot(&state);

        assert_eq!(snapshot.discovered, 1);
        assert!(snapshot.sensors["28-000000000001"].up);
    }

    #[tokio::test]
    async fn removes_stale_temperature_when_sensor_disappears() {
        let directory = TempDir::new().expect("temporary directory");
        let sensor_path = write_sensor(directory.path(), "28-000000000001", VALID_READING);
        let state = new_shared_snapshot();
        poll_once(directory.path(), &state)
            .await
            .expect("first poll should succeed");

        fs::remove_dir_all(sensor_path).expect("remove sensor");
        let report = poll_once(directory.path(), &state)
            .await
            .expect("second poll should succeed");
        let current = snapshot(&state);
        let reading = current
            .sensors
            .get("28-000000000001")
            .expect("known sensor should be retained");

        assert_eq!(current.discovered, 0);
        assert!(!reading.up);
        assert_eq!(reading.temperature_celsius, None);
        assert_eq!(report.failures[0].error_type, "not_found");

        poll_once(directory.path(), &state)
            .await
            .expect("third poll should succeed");
        assert!(snapshot(&state).sensors.is_empty());
    }

    #[tokio::test]
    async fn reports_discovery_failure_once_before_pruning_sensors() {
        let directory = TempDir::new().expect("temporary directory");
        write_sensor(directory.path(), "28-000000000001", VALID_READING);
        let state = new_shared_snapshot();
        poll_once(directory.path(), &state)
            .await
            .expect("first poll should succeed");

        fs::remove_dir_all(directory.path()).expect("remove device root");
        let error = poll_once(directory.path(), &state)
            .await
            .expect_err("discovery should fail");

        assert_eq!(error.report.failures.len(), 1);
        assert_eq!(error.report.failures[0].error_type, "io");
        assert_eq!(error.report.transitions.len(), 1);
        assert!(!error.report.transitions[0].up);
        assert!(!snapshot(&state).sensors["28-000000000001"].up);

        let error = poll_once(directory.path(), &state)
            .await
            .expect_err("discovery should keep failing");
        assert!(error.report.failures.is_empty());
        assert!(snapshot(&state).sensors.is_empty());
    }

    fn write_sensor(root: &std::path::Path, id: &str, reading: &str) -> std::path::PathBuf {
        let sensor = root.join(id);
        fs::create_dir(&sensor).expect("sensor directory");
        fs::write(sensor.join("w1_slave"), reading).expect("sensor reading");
        sensor
    }
}
