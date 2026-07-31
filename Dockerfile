# syntax=docker/dockerfile:1

FROM rust:1.94.1-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55 AS builder
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --locked --release \
    && cp /build/target/release/temperature-sensor /tmp/temperature-sensor

FROM debian:bookworm-slim@sha256:4724b8cc51e33e398f0e2e15e18d5ec2851ff0c2280647e1310bc1642182655d AS runtime

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /tmp/temperature-sensor /usr/local/bin/temperature-sensor

USER 65532:65532
EXPOSE 9100

ENTRYPOINT ["/usr/local/bin/temperature-sensor"]
