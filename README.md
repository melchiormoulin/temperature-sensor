# Fishtank Temperature Sensor

A Rust service for monitoring DS18B20 temperature probes through the Linux
1-Wire interface. It:

- discovers attached probes on every polling cycle;
- validates each reading before publishing it;
- exports metrics through Prometheus or OTLP over HTTP/protobuf;
- exposes a process liveness endpoint.

Metric definitions and Rust instrument constructors are generated from an
[OpenTelemetry Weaver](https://github.com/open-telemetry/weaver) registry that
imports the official `hw.temperature` and `hw.errors` semantic conventions.
See the [generated metric reference](docs/generated/metrics.md) for instrument
and attribute details.

## Contents

- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Metrics and Health](#metrics-and-health)
- [Container](#container)
- [Hardware Setup](#hardware-setup)
- [Development](#development)

## Quick Start

The defaults target a Raspberry Pi with 1-Wire enabled. The service reads
`/sys/bus/w1/devices` every five seconds and exports metrics to an OTLP
collector at `http://localhost:4318`:

```bash
cargo run --release
```

To expose Prometheus metrics on port `9100` instead:

```bash
OTEL_METRICS_EXPORTER=prometheus cargo run --release
```

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

Compose mounts the host 1-Wire directory at `/devices` and configures the
service to read from that path:

```bash
cp .env.example .env
docker compose up --detach --build
```

The image runs as numeric user `65532`, drops all Linux capabilities, and only
needs read-only access to the device directory. Privileged mode is not
required.

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

Read its raw value:

```bash
cat /sys/bus/w1/devices/28-*/w1_slave
```

The first line must end with `YES`. The value after `t=` is the temperature in
thousandths of a degree Celsius; for example, `t=24562` is `24.562 C`.

This command prints all detected probes in degrees Celsius without installing
another library:

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

### Troubleshooting

- IDs beginning with `00-` are false devices caused by a missing pull-up,
  poor contact, a short, or an incorrectly connected data line.
- If only `w1_bus_master1` appears, check sensor power, the adapter terminals,
  and the connection from `DAT` to physical pin 7.
- A first line ending in `NO` indicates a CRC error, usually caused by a loose
  connection or electrical noise.
- Run `pinout` on the Pi to display the physical header orientation and BCM
  GPIO numbers.

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
