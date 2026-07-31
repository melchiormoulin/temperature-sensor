# Fishtank Temperature Sensor

A Rust service that automatically discovers Linux 1-Wire DS18B20 probes,
validates each reading, and exports the result through either a Prometheus
scrape endpoint or OTLP HTTP/protobuf.

Metric definitions and Rust instrument constructors are generated from an
[OpenTelemetry Weaver](https://github.com/open-telemetry/weaver) registry. The
registry imports the official OpenTelemetry `hw.temperature` and `hw.errors`
semantic conventions.

## Run Locally

The defaults match a Raspberry Pi with 1-Wire enabled and use the standard OTLP
metrics exporter:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
cargo run --release
```

To expose Prometheus metrics instead:

```bash
OTEL_METRICS_EXPORTER=prometheus cargo run --release
```

The application exits at startup if the device root is missing or unreadable.
It is valid for the directory to contain no sensors; newly attached probes are
discovered during later polling cycles.

```text
Usage: temperature-sensor [OPTIONS]

Options:
      --listen <LISTEN>                [default: 0.0.0.0:9100]
      --device-root <DEVICE_ROOT>      [default: /sys/bus/w1/devices]
      --poll-interval <POLL_INTERVAL>  [default: 5s]
  -h, --help
  -V, --version
```

Durations accept values such as `500ms`, `5s`, and `1m`. A zero polling
interval is rejected.

### OpenTelemetry Configuration

Exporter configuration uses the standard OpenTelemetry environment variables
instead of a project-specific configuration file:

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

Exporter names are case-insensitive. An empty selector uses the standard `otlp`
default; an unsupported selector logs a warning and falls back to `otlp`. The
OTLP protocol is intentionally fixed to HTTP/protobuf. Do not put secrets in
command-line flags because command lines are visible through process inspection;
use `OTEL_EXPORTER_OTLP_HEADERS` or inject credentials through the deployment
platform.

## Endpoints

| Endpoint | Description |
|---|---|
| `GET /metrics` | Prometheus text exposition when `OTEL_METRICS_EXPORTER=prometheus` |
| `GET /healthz` | Process liveness |

Verify liveness, and verify metrics when Prometheus is selected:

```bash
curl --fail http://localhost:9100/healthz
curl --fail http://localhost:9100/metrics
```

The OpenTelemetry Prometheus exporter translates semantic names and units:

| OTLP metric | Prometheus metric | Description |
|---|---|---|
| `hw.temperature` | `hw_temperature_celsius` | Last valid temperature by `hw_id` |
| `hw.errors` | `hw_errors_total` | Read and discovery errors by `hw_id`, `hw_type`, and `error_type` |
| `fishtank.sensor.up` | `fishtank_sensor_up_ratio` | Most recent read status by `hw_id` |
| `fishtank.sensor.count` | `fishtank_sensor_count` | Currently discovered probe count |

Invalid CRC, malformed, out-of-range, timed-out, and I/O readings do not publish
a stale temperature. A disappeared probe remains represented with
`fishtank_sensor_up_ratio 0` for one polling cycle and is then pruned. Error
types are bounded to `crc`, `format`, `io`, `not_found`, `range`, and `timeout`.

Example Prometheus scrape configuration:

```yaml
scrape_configs:
  - job_name: fishtank-temperature
    static_configs:
      - targets: ["raspberry-pi:9100"]
```

## Container

The image runs as numeric user `65532`, has no Linux capabilities, and only
needs read-only access to the 1-Wire device directory. Compose mounts the host
directory at `/devices` and passes `--device-root /devices`; privileged mode is
not required.

```bash
cp .env.example .env
docker compose up --detach --build
```

The example environment selects Prometheus. To use OTLP, set
`OTEL_METRICS_EXPORTER=otlp` and set `OTEL_EXPORTER_OTLP_ENDPOINT` to a collector
reachable from the container.

`localhost` inside the container refers to the container itself. Use the
collector's Compose service name, host gateway, or LAN address as appropriate.
The same Dockerfile builds natively on Raspberry Pi OS. Cross-build an ARM64
image from another host with:

```bash
docker buildx build --platform linux/arm64 --tag temperature-sensor:local --load .
```

## Weaver Workflow

The application telemetry registry is under `registry/`. It uses Weaver schema
format `definition/2`, which Weaver `0.25.1` still labels experimental. Both the
Weaver container image version and the OpenTelemetry semantic-conventions
dependency are pinned to keep generation reproducible. Docker is the only
Weaver prerequisite; no host Weaver installation is needed.

Run:

```bash
make registry-check
make generate
```

`make generate` validates project policies, regenerates `src/generated/`, runs
Rustfmt on generated Rust, and updates `docs/generated/metrics.md`. Generated
files are committed so production builds never need Weaver or network access.
CI uses the same Docker image to regenerate them and fails on drift.

## Development Checks

```bash
make check
```

This runs Rustfmt, Clippy with warnings denied, all tests, and a locked release
build. Tests cover DS18B20 parsing and discovery, Prometheus exposition, and an
actual OTLP HTTP/protobuf export to a local receiver.

## Hardware Installation

### Prerequisites

- Raspberry Pi 4 with the 40-pin `J8` GPIO header
- Raspberry Pi OS and a user with `sudo` access
- One probe and adapter from the [BTFO DS18B20 kit](https://www.amazon.fr/dp/B0GT3XBDXM)
- One supplied 4.7 kOhm resistor
- Three female jumper wires or a breadboard

Raspberry Pi OS already includes the Linux 1-Wire driver and Python 3. No
third-party Python package is required.

### Working Pin Layout

Power off and unplug the Pi before changing GPIO connections:

```bash
sudo poweroff
```

View the Pi from above with the Ethernet/USB connectors on the right and the
USB-C/micro-HDMI connectors along the bottom. The row nearest the centre of
the board is the bottom row in this diagram:

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
