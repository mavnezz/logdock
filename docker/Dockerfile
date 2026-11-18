# syntax=docker/dockerfile:1
#
# Logdock as one image: the web interface built with Node, the server with
# Rust, and a runtime image with nothing but the two (docs/architecture.md,
# 2.3). Build from the repository's root:
#
#   docker build -f docker/Dockerfile -t logdock:local .

FROM node:24-trixie-slim AS web
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

FROM rust:1.99.1-slim-trixie AS server
# The version the image brings, without rust-toolchain.toml's components.
ENV RUSTUP_TOOLCHAIN=1.99.1
ARG LOGDOCK_VERSION
ENV LOGDOCK_VERSION=${LOGDOCK_VERSION}
WORKDIR /src/backend
COPY backend/ ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/backend/target \
    cargo build --release --locked -p logdockd \
    && cp target/release/logdockd /usr/local/bin/logdockd

# glibc and CA certificates (for mail over TLS), no shell, user 65532.
FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=server /usr/local/bin/logdockd /usr/local/bin/logdockd
COPY --from=web /src/frontend/dist /usr/share/logdock/web
ENV LOGDOCK_DATA_DIR=/data \
    LOGDOCK_WEB_DIR=/usr/share/logdock/web \
    LOGDOCK_LISTEN=0.0.0.0:8080
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 CMD ["/usr/local/bin/logdockd", "health"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/logdockd"]
CMD ["serve"]
