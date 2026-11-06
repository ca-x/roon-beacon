FROM node:24-bookworm-slim AS frontend

WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run check && npm run build

FROM rust:1-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY vendor ./vendor
COPY --from=frontend /build/crates/roon-beacon/src/web ./crates/roon-beacon/src/web
RUN cargo build --locked --release --bin roon-beacon

FROM debian:bookworm-slim

LABEL org.opencontainers.image.source="https://github.com/ca-x/roon-beacon" \
    org.opencontainers.image.licenses="GPL-3.0-only"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        avahi-daemon ca-certificates dbus ffmpeg shairport-sync tini util-linux wget \
    && rm -rf /var/lib/apt/lists/* \
    && (shairport-sync -h 2>&1 || true) | grep -q stdout

COPY --from=builder /build/target/release/roon-beacon /usr/local/bin/roon-beacon
COPY docker/entrypoint.sh /usr/local/bin/docker-entrypoint.sh
COPY docker/avahi-daemon.conf /etc/avahi/avahi-daemon.conf
COPY LICENSE /usr/share/doc/roon-beacon/LICENSE
COPY web/THIRD_PARTY_NOTICES.md /usr/share/doc/roon-beacon/THIRD_PARTY_NOTICES.md
COPY vendor/roon-api/LICENSE-MIT /usr/share/doc/roon-beacon/ROON-SDK-LICENSE-MIT
COPY vendor/roon-api/LICENSE-APACHE /usr/share/doc/roon-beacon/ROON-SDK-LICENSE-APACHE
RUN chmod 755 /usr/local/bin/docker-entrypoint.sh \
    && install -d -m 700 -o shairport-sync -g shairport-sync /data \
    && rm -f /etc/machine-id /var/lib/dbus/machine-id

ENV DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket
VOLUME ["/data"]
EXPOSE 9339/tcp
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD wget -q --spider http://127.0.0.1:9339/api/health || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]
CMD ["--web-bind", "0.0.0.0:9339", "--data-dir", "/data", "--enable-audio"]
