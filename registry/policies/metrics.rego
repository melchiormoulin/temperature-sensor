package after_resolution

import rego.v1

metric_violation(id, description, metric_name) := violation if {
    violation := {
        "id": id,
        "type": "semconv_group",
        "category": "metric",
        "group": metric_name,
        "attr": "",
        "description": description,
    }
}

deny contains metric_violation(
    "invalid_fishtank_metric_name",
    sprintf("Application metric name %q must use lowercase dot-separated words.", [metric.name]),
    metric.name,
) if {
    metric := input.registry.metrics[_]
    startswith(metric.name, "fishtank.")
    not regex.match(`^fishtank(\.[a-z][a-z0-9_]*)+$`, metric.name)
}

deny contains metric_violation(
    "metric_namespace_collision",
    sprintf("Metric %q is also used as a namespace by %q.", [metric.name, other.name]),
    metric.name,
) if {
    metric := input.registry.metrics[_]
    other := input.registry.metrics[_]
    metric.name != other.name
    startswith(other.name, concat("", [metric.name, "."]))
}

deny contains metric_violation(
    "missing_rust_metric_value_type",
    sprintf("Metric %q must declare annotations.code_generation.metric_value_type.", [metric.name]),
    metric.name,
) if {
    metric := input.registry.metrics[_]
    not metric.annotations.code_generation.metric_value_type
}
