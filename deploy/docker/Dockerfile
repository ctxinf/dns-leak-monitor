# syntax=docker/dockerfile:1
FROM rust:1.95-bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends libpcap-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY static ./static
RUN cargo test --locked && cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends libpcap0.8 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /etc/dns-leak-monitor /var/lib/dns-leak-monitor
COPY --from=builder /build/target/release/dns-leak-monitor /usr/local/bin/dns-leak-monitor
COPY deploy/docker/config/config.toml /usr/share/dns-leak-monitor/config.toml
COPY --chmod=0755 deploy/docker/entrypoint.sh /usr/local/bin/docker-entrypoint.sh
USER 0:0
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["--config", "/etc/dns-leak-monitor/config.toml"]
