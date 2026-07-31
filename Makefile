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

.PHONY: all check docker-build docker-run generate check-generated registry-check

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

generate: registry-check
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

check-generated: generate
	git diff --exit-code -- src/generated docs/generated

check:
	cargo fmt --check
	cargo clippy --locked --all-targets --all-features -- -D warnings
	cargo test --locked --all-targets
	cargo build --locked --release
