# syntax=docker/dockerfile:1

FROM oven/bun:1.4.0 AS bun-runtime

FROM node:22-bookworm AS web-builder
COPY --from=bun-runtime /usr/local/bin/bun /usr/local/bin/bun
WORKDIR /src
COPY package.json bun.lock bunfig.toml ./
COPY apps/web/package.json apps/web/package.json
COPY apps/desktop/package.json apps/desktop/package.json
RUN bun install --frozen-lockfile
COPY apps/web apps/web
COPY scripts/check-web-ui-contract.mjs scripts/check-web-ui-contract.mjs
COPY scripts/check-web-bundle-budget.mjs scripts/check-web-bundle-budget.mjs
RUN cd apps/web \
    && ../../node_modules/.bin/tsc -b \
    && bun ../../scripts/check-web-ui-contract.mjs \
    && node ../../node_modules/vite/bin/vite.js build \
    && bun ../../scripts/check-web-bundle-budget.mjs

FROM rust:1.88-bookworm AS rust-builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY eval eval
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin cortana \
    && cp /src/target/release/cortana /usr/local/bin/cortana

FROM python:3.11-slim-bookworm AS runtime
ARG CORTANA_VERSION=dev
LABEL org.opencontainers.image.title="Cortana" \
      org.opencontainers.image.description="Local-first single-node ContextProvider" \
      org.opencontainers.image.source="https://github.com/0xPlayerOne/cortana" \
      org.opencontainers.image.version="${CORTANA_VERSION}" \
      org.opencontainers.image.licenses="Apache-2.0"

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl tini \
    && groupadd --gid 10001 cortana \
    && useradd --uid 10001 --gid cortana --create-home --home-dir /home/cortana cortana

WORKDIR /opt/cortana
COPY pyproject.toml README.md ./
COPY src/cortana src/cortana
RUN python -m pip install --no-cache-dir ".[ingestion]"
COPY --from=rust-builder /usr/local/bin/cortana /usr/local/bin/cortana
COPY --from=web-builder /src/apps/web/dist /opt/cortana/web

RUN install -d -o 10001 -g 10001 \
      /etc/cortana \
      /var/lib/cortana \
      /var/lib/cortana/backups \
      /var/cache/cortana/models

VOLUME ["/var/lib/cortana", "/var/lib/cortana/backups", "/var/cache/cortana/models"]
EXPOSE 7331
USER 10001:10001
ENTRYPOINT ["/usr/bin/tini", "--", "cortana"]
CMD ["--config", "/etc/cortana/config.toml", "serve", "--address", "0.0.0.0:7331", "--web-dir", "/opt/cortana/web", "--allow-remote"]
