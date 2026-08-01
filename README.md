# Fishtank Temperature Sensor

A hands-on tutorial for reading a DS18B20 temperature probe on a Raspberry Pi
through the Linux 1-Wire interface. It starts with the raw sensor file, then
builds a Rust service that:

- discovers attached probes on every polling cycle;
- validates each reading before publishing it;
- exports metrics through Prometheus or OTLP over HTTP/protobuf;
- exposes a process liveness endpoint.

You do not need a local Rust toolchain. Docker builds and runs the monitoring
service after the hardware is ready.

## Contents

- [Hardware Setup](#hardware-setup)
- [Read a Temperature](#read-a-temperature)
- [Run the Service](#run-the-service)
- [Configuration](#configuration)
- [Metrics and Health](#metrics-and-health)
- [Container](#container)
- [How It Works](#how-it-works)
- [Development](#development)

## Hardware Setup

### Requirements

- Raspberry Pi with the 40-pin `J8` GPIO header (tested on Raspberry Pi 4)
- Raspberry Pi OS and a user with `sudo` access
- One DS18B20 probe and adapter, tested with the
  [BTFO DS18B20 kit](https://www.amazon.fr/dp/B0GT3XBDXM)
- One 4.7 kOhm pull-up resistor
- Three female jumper wires or a breadboard

Raspberry Pi OS already includes the Linux 1-Wire driver and Python 3. No
third-party Python package is required.

### Wire the Probe

Power off and unplug the Pi before changing GPIO connections:

```bash
sudo poweroff
```

View the Pi from above with the Ethernet/USB connectors on the right and the
USB-C/micro-HDMI connectors along the bottom. The row nearest the centre of the
board is the bottom row in this diagram:

```text
Outside/top edge of the board

     Position 1   Position 2   Position 3   Position 4   Position 5
      Pin 2        Pin 4        Pin 6        Pin 8        Pin 10
       5V           5V           GND          GPIO14       GPIO15
       [ ]          [ ]          [ ]          [ ]          [ ]
       [ ]          [ ]          [ ]          [ ]          [ ]
      Pin 1        Pin 3        Pin 5        Pin 7        Pin 9
       3V3          GPIO2        GPIO3        GPIO4        GND
       VCC                                      DAT          GND

Inside/centre of the board (bottom row)
```

| Probe connection | Bottom-row position | Physical pin | Raspberry Pi function |
|---|---:|---:|---|
| `VCC` (red) | 1 | 1 | 3.3 V |
| `DAT` (yellow) | 4 | 7 | BCM GPIO4 / 1-Wire data |
| `GND` (black) | 5 | 9 | Ground |

Install the 4.7 kOhm resistor between `VCC` and `DAT`. It is a parallel
pull-up, not a resistor placed in series with the data wire:

```text
Physical pin 1 (3.3 V) --------+-------- VCC
                                |
                              4.7 kOhm
                                |
Physical pin 7 (GPIO4) --------+-------- DAT

Physical pin 9 (GND) ------------------- GND
```

Use 3.3 V, not 5 V. The Raspberry Pi GPIO input is not 5 V tolerant. Follow
the adapter labels because the colours of the supplied jumper wires may vary.

For two probes, connect both probes in parallel to the same `VCC`, `DAT`, and
`GND` lines. Only one 4.7 kOhm pull-up resistor is required for the bus.

### Enable 1-Wire

Boot the Pi, update the package index, enable 1-Wire, and reboot:

```bash
sudo apt update
sudo raspi-config nonint do_onewire 0
sudo reboot
```

The interactive alternative is `sudo raspi-config`, then **Interface Options
> 1-Wire > Yes**.

### Verify the Probe

After reboot, list the 1-Wire devices:

```bash
ls /sys/bus/w1/devices/
```

A working DS18B20 appears with a family ID beginning with `28-`:

```text
28-000000000001  w1_bus_master1
```

The `28-000000000001` value is the example probe's unique sensor ID. Your ID
will be different.

## Read a Temperature

The Linux driver exposes each probe as a text file. Inspect the raw reading:

```bash
cat /sys/bus/w1/devices/28-*/w1_slave
```

Example output:

```text
9e 01 4b 46 7f ff 02 10 56 : crc=56 YES
9e 01 4b 46 7f ff 02 10 56 t=24562
```

The first line must end with `YES`, which means the CRC check passed. The value
after `t=` is the temperature in thousandths of a degree Celsius, so `t=24562`
is `24.562 C`.

This Python snippet prints every detected probe in degrees Celsius without
installing another library:

```bash
python3 - <<'PY'
from pathlib import Path

for sensor in sorted(Path("/sys/bus/w1/devices").glob("28-*")):
    lines = (sensor / "w1_slave").read_text().splitlines()
    if lines[0].strip().endswith("YES") and "t=" in lines[1]:
        temperature = int(lines[1].split("t=")[1]) / 1000
        print(f"{sensor.name}: {temperature:.3f} C")
    else:
        print(f"{sensor.name}: read error")
PY
```

Expected output resembles:

```text
28-000000000001: 24.562 C
```

### Troubleshooting

- IDs beginning with `00-` are false devices caused by a missing pull-up,
  poor contact, a short, or an incorrectly connected data line.
- If only `w1_bus_master1` appears, check sensor power, the adapter terminals,
  and the connection from `DAT` to physical pin 7.
- A first line ending in `NO` indicates a CRC error, usually caused by a loose
  connection or electrical noise.
- Run `pinout` on the Pi to display the physical header orientation and BCM
  GPIO numbers.

## Run the Service

Once the raw reading works, the Rust service can poll it continuously and
publish validated readings as metrics.

### Install Docker

Install Docker Engine and the Compose plugin using Docker's official
[Raspberry Pi OS instructions](https://docs.docker.com/engine/install/raspberry-pi-os/).
Complete the
[Linux post-installation steps](https://docs.docker.com/engine/install/linux-postinstall/)
if you want to run Docker without `sudo`, then verify the installation:

```bash
docker --version
docker compose version
```

### Run with the Probe

From the repository root, copy the example environment and start the service.
The example selects Prometheus so the result can be inspected locally. The
first image build can take several minutes on a Raspberry Pi:

```bash
cp .env.example .env
docker compose up --build
```

Compose mounts `/sys/devices/w1_bus_master1` into the container at `/devices`
with read-only access. That is the directory holding each probe as a real
directory. `/sys/bus/w1/devices` contains only relative symlinks into it, and
bind-mounting that path alone makes them resolve into a loop inside the
container, so reads fail with `ELOOP`.

In another terminal, check the process and find the temperature metric:

```bash
curl --fail http://localhost:9100/healthz
curl --silent http://localhost:9100/metrics | grep hw_temperature_celsius
```

The metric includes the same sensor ID and temperature seen in the raw file:

```text
hw_temperature_celsius{hw_id="28-000000000001"} 24.562
```

Press `Ctrl+C` to stop the service.

### Try without Hardware

The committed fixture lets you run the complete container on a Linux
development machine without a probe:

```bash
docker build --tag temperature-sensor:local .
docker run --rm \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --publish 127.0.0.1:9100:9100 \
  --volume "$PWD/tests/fixtures/devices:/devices:ro" \
  --env OTEL_METRICS_EXPORTER=prometheus \
  temperature-sensor:local --device-root /devices
```

It exposes the example sensor `28-000000000001` at `24.562 C`.

The service exits at startup if the device root is missing or unreadable. An
empty directory is valid; probes attached later are discovered on a subsequent
polling cycle.

## Configuration

### Command-Line Options

| Option | Default | Description |
|---|---|---|
| `--listen <ADDRESS>` | `0.0.0.0:9100` | Address for health and optional Prometheus HTTP endpoints |
| `--device-root <PATH>` | `/sys/bus/w1/devices` | Directory containing `28-*` 1-Wire devices |
| `--poll-interval <DURATION>` | `5s` | Delay between sensor polling cycles |
| `-h`, `--help` | | Print command help |
| `-V`, `--version` | | Print the service version |

Durations accept values such as `500ms`, `5s`, and `1m`. The polling interval
must be greater than zero.

### OpenTelemetry

Exporter configuration uses standard OpenTelemetry environment variables:

| Variable | Purpose | Default |
|---|---|---|
| `OTEL_METRICS_EXPORTER` | Metrics exporter: `otlp`, `prometheus`, or `none` | `otlp` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Generic OTLP base URL; `/v1/metrics` is appended | `http://localhost:4318` |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | Metrics-specific full endpoint URL | unset |
| `OTEL_EXPORTER_OTLP_HEADERS` | URL-encoded authentication or routing headers | unset |
| `OTEL_EXPORTER_OTLP_METRICS_TIMEOUT` | Export timeout in milliseconds | `10000` |
| `OTEL_METRIC_EXPORT_INTERVAL` | OTLP push interval in milliseconds | `60000` |
| `OTEL_SERVICE_NAME` | OpenTelemetry resource service name | `temperature-sensor` |
| `OTEL_RESOURCE_ATTRIBUTES` | Additional resource attributes | unset |
| `RUST_LOG` | Rust tracing filter | `info` |

Exporter values are case-insensitive. An empty selector uses the standard
`otlp` default; an unsupported value logs a warning and falls back to `otlp`.
The OTLP protocol is fixed to HTTP/protobuf.

Do not put secrets in command-line flags because command lines are visible
through process inspection. Use `OTEL_EXPORTER_OTLP_HEADERS` or inject
credentials through the deployment platform.

## Metrics and Health

| Endpoint | Availability | Description |
|---|---|---|
| `GET /healthz` | Always | Process liveness |
| `GET /metrics` | Prometheus exporter only | Prometheus text exposition |

With the Prometheus exporter selected, verify both endpoints with:

```bash
curl --fail http://localhost:9100/healthz
curl --fail http://localhost:9100/metrics
```

The OpenTelemetry Prometheus exporter translates OTLP names and units as
follows:

| OTLP metric | Prometheus metric | Description |
|---|---|---|
| `hw.temperature` | `hw_temperature_celsius` | Last valid temperature by `hw_id` |
| `hw.errors` | `hw_errors_total` | Read and discovery errors by `hw_id`, `hw_type`, and `error_type` |
| `fishtank.sensor.up` | `fishtank_sensor_up_ratio` | Most recent read status by `hw_id` |
| `fishtank.sensor.count` | `fishtank_sensor_count` | Currently discovered probe count |

Invalid CRC, malformed, out-of-range, timed-out, and I/O readings never publish
a stale temperature. A disappeared probe reports `fishtank_sensor_up_ratio 0`
for one polling cycle before it is pruned. Error types are bounded to `crc`,
`format`, `io`, `not_found`, `range`, and `timeout`.

Example Prometheus scrape configuration:

```yaml
scrape_configs:
  - job_name: fishtank-temperature
    static_configs:
      - targets: ["raspberry-pi:9100"]
```

## Container

For a long-running deployment, start Compose in the background and follow its
logs:

```bash
docker compose up --detach --build
docker compose logs --follow
```

The Compose configuration mounts the host 1-Wire master directory at
`/devices`. The image runs as numeric user `65532`, drops all Linux
capabilities, and only needs read-only access to that directory. Privileged
mode is not required.

The example environment selects Prometheus. To use OTLP, set
`OTEL_METRICS_EXPORTER=otlp` and point `OTEL_EXPORTER_OTLP_ENDPOINT` at a
collector reachable from the container.

`localhost` inside the container refers to the container itself. Use the
collector's Compose service name, host gateway, or LAN address as appropriate.

The Dockerfile builds natively on Raspberry Pi OS. To cross-build an ARM64
image from another host:

```bash
docker buildx build --platform linux/arm64 --tag temperature-sensor:local --load .
```

### Rootless Podman

Rootless Podman runs this stack unchanged apart from two differences, both
verified against Podman `5.4.2` and podman-compose `1.3.0`.

Boot persistence needs a different restart policy. `podman-restart.service`
starts containers with `podman start --all --filter restart-policy=always`,
and that filter does not match the `unless-stopped` policy in `compose.yaml`,
so the container stays stopped after a reboot. Override it in a
`compose.override.yaml` beside `compose.yaml`, which leaves the Docker
behaviour untouched:

```yaml
services:
  temperature-sensor:
    restart: always
```

Rootless containers also need a user manager that outlives the login session,
so enable lingering and the restart unit once:

```bash
loginctl enable-linger
systemctl --user enable --now podman-restart.service
```

Note that `always` restarts a container you stopped deliberately the next time
the machine boots, which `unless-stopped` does not.

podman-compose does not apply `${VARIABLE:-default}` fallbacks. Any variable
left unset in `.env` reaches the container as that literal string, which shows
up in exported metrics as `service_name="${OTEL_SERVICE_NAME:-...}"`. Set the
defaults explicitly in `.env`:

```bash
OTEL_SERVICE_NAME=fishtank-temperature-sensor
OTEL_EXPORTER_OTLP_METRICS_TIMEOUT=10000
OTEL_EXPORTER_OTLP_HEADERS=
```

## How It Works

The Linux kernel handles the 1-Wire protocol and presents each probe as a
`w1_slave` file. The application only needs to discover, read, validate, and
publish those files:

```text
DS18B20 probe
    |
Linux 1-Wire drivers
    |
/sys/bus/w1/devices/28-*/w1_slave
    |
src/sensor.rs -> shared sensor snapshot -> src/telemetry.rs
                                             |        |
                                      Prometheus     OTLP
                                       /metrics    collector
```

### Source Guide

| Path | Responsibility |
|---|---|
| [`src/main.rs`](src/main.rs) | Initializes logging, parses arguments, and starts the application |
| [`src/cli.rs`](src/cli.rs) | Defines the listen address, device root, and polling interval options |
| [`src/app.rs`](src/app.rs) | Runs the polling loop, HTTP server, and graceful shutdown |
| [`src/sensor.rs`](src/sensor.rs) | Discovers probes, validates readings, and maintains the current snapshot |
| [`src/telemetry.rs`](src/telemetry.rs) | Converts the snapshot and failures into OpenTelemetry metrics |
| [`src/http.rs`](src/http.rs) | Serves `/healthz` and, when enabled, `/metrics` |
| [`registry/`](registry/) | Defines the metric schema used to generate Rust instruments and documentation |

### One Polling Cycle

1. `discover` lists the device root and keeps entries whose IDs begin with the
   DS18B20 family prefix `28-`.
2. `read_temperature` reads each `w1_slave` file with a two-second timeout.
3. `parse_temperature` requires a successful `YES` CRC result, finds the `t=`
   field, parses it as an integer, and checks the DS18B20 range of `-55 C` to
   `125 C`.
4. The millidegree value is divided by `1000` to produce degrees Celsius.
5. `poll_once` replaces the shared snapshot. Failed readings do not retain an
   old temperature, and disappeared probes are reported unavailable before
   being pruned.
6. OpenTelemetry instruments observe the snapshot for Prometheus or OTLP, and
   read failures increment `hw.errors`.

Metric definitions and Rust instrument constructors are generated from an
[OpenTelemetry Weaver](https://github.com/open-telemetry/weaver) registry that
imports the official `hw.temperature` and `hw.errors` semantic conventions.
See the [generated metric reference](docs/generated/metrics.md) for instrument
and attribute details.

## Development

### Checks

```bash
make check
```

This runs rustfmt, Clippy with warnings denied, all tests, and a locked release
build. Tests cover DS18B20 parsing and discovery, Prometheus exposition, and an
actual OTLP export over HTTP/protobuf to a local receiver.

### Generate Telemetry Code and Documentation

The telemetry registry is under `registry/`. It uses Weaver schema format
`definition/2`, which Weaver `0.25.1` still labels experimental. The Weaver
container image and OpenTelemetry semantic-conventions dependency are pinned
for reproducible generation. Docker is the only prerequisite; no host Weaver
installation is needed.

```bash
make registry-check
make generate
```

`make generate` validates project policies, regenerates `src/generated/`, runs
rustfmt on generated Rust, and updates `docs/generated/metrics.md`. Generated
files are committed so production builds never require Weaver or network
access. CI regenerates them with the same container image and fails on drift.
