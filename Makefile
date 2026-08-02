CONTAINER_ENGINE ?= docker
CONTAINER_ENGINE_VERSION := $(shell $(CONTAINER_ENGINE) --version 2>/dev/null)
CONTAINER_IMAGE ?= temperature-sensor:local
CONTAINER_RUN_ARGS ?=
HOST_DEVICE_ROOT ?= /sys/bus/w1/devices
ifneq (,$(findstring podman,$(CONTAINER_ENGINE_VERSION)))
WEAVER_RUN_ARGS ?= --userns=keep-id
WEAVER_VOLUME_SUFFIX ?= :Z
else
WEAVER_RUN_ARGS ?=
WEAVER_VOLUME_SUFFIX ?=
endif
WEAVER_VERSION := 0.25.1
WEAVER_IMAGE := docker.io/otel/weaver:v$(WEAVER_VERSION)
WEAVER := $(CONTAINER_ENGINE) run --rm $(WEAVER_RUN_ARGS) \
	--user "$(shell id -u):$(shell id -g)" \
	--env HOME=/tmp \
	--volume "$(CURDIR):/home/weaver$(WEAVER_VOLUME_SUFFIX)" \
	--workdir /home/weaver \
	"$(WEAVER_IMAGE)"
WEAVER_COMMON := registry
SLOTH_VERSION := 0.16.0
SLOTH_IMAGE := ghcr.io/slok/sloth:v$(SLOTH_VERSION)
SLOTH := $(CONTAINER_ENGINE) run --rm $(WEAVER_RUN_ARGS) \
	--user "$(shell id -u):$(shell id -g)" \
	--volume "$(CURDIR):/work$(WEAVER_VOLUME_SUFFIX)" \
	--workdir /work \
	"$(SLOTH_IMAGE)"

.PHONY: all check docker-build docker-run generate check-generated registry-check sloth-check

all: check

docker-build:
	$(CONTAINER_ENGINE) build --tag "$(CONTAINER_IMAGE)" .

docker-run:
	$(CONTAINER_ENGINE) run --rm $(CONTAINER_RUN_ARGS) \
		--read-only \
		--cap-drop ALL \
		--security-opt no-new-privileges \
		--publish 9100:9100 \
		--volume "$(HOST_DEVICE_ROOT):/devices:ro" \
		"$(CONTAINER_IMAGE)" \
		--device-root /devices

registry-check:
	@version="$$($(WEAVER) --version)"; \
	case "$$version" in \
		"weaver $(WEAVER_VERSION)") ;; \
		*) echo "expected weaver $(WEAVER_VERSION), found $$version" >&2; exit 1 ;; \
	esac
	$(WEAVER) $(WEAVER_COMMON) check \
		--registry registry \
		--v2 \
		--policy registry/policies

sloth-check:
	@version="$$($(SLOTH) version)"; \
	case "$$version" in \
		"v$(SLOTH_VERSION)") ;; \
		*) echo "expected sloth v$(SLOTH_VERSION), found $$version" >&2; exit 1 ;; \
	esac

generate: registry-check sloth-check
	$(WEAVER) $(WEAVER_COMMON) generate \
		--registry registry \
		--templates templates \
		--v2 \
		--policy registry/policies \
		rust src/generated
	cargo fmt -- src/generated/attributes.rs src/generated/metrics.rs src/generated/schema.rs
	$(WEAVER) $(WEAVER_COMMON) generate \
		--registry registry \
		--templates templates \
		--v2 \
		--policy registry/policies \
		markdown docs/generated
	$(WEAVER) $(WEAVER_COMMON) generate \
		--registry registry \
		--templates templates \
		--v2 \
		--policy registry/policies \
		sloth monitoring/generated
	$(SLOTH) validate --no-color --input monitoring/generated/sloth.yaml
	$(SLOTH) generate --no-color \
		--input monitoring/generated/sloth.yaml \
		--out monitoring/generated/prometheus-rules.yaml

check-generated: generate
	git diff --exit-code -- src/generated docs/generated monitoring/generated
	@untracked="$$(git ls-files --others --exclude-standard -- src/generated docs/generated monitoring/generated)"; \
	if [ -n "$$untracked" ]; then \
		printf 'untracked generated files:\n%s\n' "$$untracked" >&2; \
		exit 1; \
	fi

check:
	cargo fmt --check
	cargo clippy --locked --all-targets --all-features -- -D warnings
	cargo test --locked --all-targets
	cargo build --locked --release
