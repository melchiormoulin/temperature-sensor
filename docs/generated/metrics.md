# Generated Metrics

This file is generated from the OpenTelemetry Weaver registry. Do not edit it directly.


## `fishtank.sensor.count`

Number of DS18B20 temperature sensors currently discovered.

| Property | Value |
|---|---|
| Instrument | `gauge` |
| Unit | `{sensor}` |
| Stability | `development` |



## `fishtank.sensor.up`

Whether the most recent read from a temperature sensor succeeded.

| Property | Value |
|---|---|
| Instrument | `gauge` |
| Unit | `1` |
| Stability | `development` |


| Attribute | Requirement |
|---|---|

| `hw.id` | `required` |



## `hw.errors`

Number of errors encountered by the component.

| Property | Value |
|---|---|
| Instrument | `counter` |
| Unit | `{error}` |
| Stability | `development` |


| Attribute | Requirement |
|---|---|

| `error.type` | `{"conditionally_required": "if and only if an error has occurred"}` |

| `hw.id` | `required` |

| `hw.name` | `recommended` |

| `hw.parent` | `recommended` |

| `hw.type` | `required` |

| `network.io.direction` | `recommended` |



## `hw.temperature`

Temperature in degrees Celsius.

| Property | Value |
|---|---|
| Instrument | `gauge` |
| Unit | `Cel` |
| Stability | `development` |


| Attribute | Requirement |
|---|---|

| `hw.id` | `required` |

| `hw.name` | `recommended` |

| `hw.parent` | `recommended` |

| `hw.sensor_location` | `recommended` |


