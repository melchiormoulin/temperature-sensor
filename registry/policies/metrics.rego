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

valid_sloth_annotation(slo) if {
    service := object.get(slo, "service", "")
    is_string(service)
    service != ""

    name := object.get(slo, "name", "")
    is_string(name)
    name != ""

    description := object.get(slo, "description", "")
    is_string(description)
    description != ""

    objective := object.get(slo, "objective", 0)
    is_number(objective)
    objective > 0
    objective < 100

    prometheus_metric := object.get(slo, "prometheus_metric", "")
    is_string(prometheus_metric)
    prometheus_metric != ""

    prometheus_selector := object.get(slo, "prometheus_selector", "")
    is_string(prometheus_selector)
    prometheus_selector != ""

    object.get(slo, "error_operator", "") == ">"

    threshold := object.get(slo, "threshold", null)
    is_number(threshold)

    alert_name := object.get(slo, "alert_name", "")
    is_string(alert_name)
    alert_name != ""

    page_severity := object.get(slo, "page_severity", "")
    is_string(page_severity)
    page_severity != ""

    ticket_severity := object.get(slo, "ticket_severity", "")
    is_string(ticket_severity)
    ticket_severity != ""
}

deny contains metric_violation(
    "invalid_sloth_annotation",
    sprintf("Metric refinement %q must declare a complete Sloth temperature SLO annotation.", [refinement.id]),
    refinement.name,
) if {
    refinement := input.refinements.metrics[_]
    slo := refinement.annotations.sloth
    not valid_sloth_annotation(slo)
}

valid_sloth_metric(refinement) if {
    refinement.instrument == "gauge"
    refinement.unit == "Cel"
}

deny contains metric_violation(
    "invalid_sloth_metric",
    sprintf("Metric refinement %q must target a Celsius gauge.", [refinement.id]),
    refinement.name,
) if {
    refinement := input.refinements.metrics[_]
    refinement.annotations.sloth
    not valid_sloth_metric(refinement)
}

deny contains metric_violation(
    "duplicate_sloth_slo_name",
    sprintf("Metric refinements %q and %q use the same Sloth SLO name %q.", [first.id, second.id, first.annotations.sloth.name]),
    first.name,
) if {
    first := input.refinements.metrics[_]
    second := input.refinements.metrics[_]
    first.id < second.id
    first.annotations.sloth.name == second.annotations.sloth.name
}

deny contains metric_violation(
    "inconsistent_sloth_service",
    sprintf("Metric refinements %q and %q use different Sloth services.", [first.id, second.id]),
    first.name,
) if {
    first := input.refinements.metrics[_]
    second := input.refinements.metrics[_]
    first.id < second.id
    first.annotations.sloth.service != second.annotations.sloth.service
}
