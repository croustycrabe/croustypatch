# CroustyPatch server image: Rust backend + the exported Next.js UI served by the same binary.
# Build from the repository root:  docker build -f docker/Dockerfile -t croustypatch-server .

FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
ENV NODE_ENV=production
RUN npx next build

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY common common
COPY server server
COPY agent agent
# libpq is compiled in (pq-sys "bundled"): no system PostgreSQL client library is needed.
RUN cargo build --release --locked -p croustypatch-server -p croustypatch-agent \
 && mkdir -p /out \
 && cp target/release/croustypatch-agent /out/croustypatch-agent-linux-$(uname -m)

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /var/lib/croustypatch --shell /usr/sbin/nologin croustypatch \
 && install -d -o croustypatch -g croustypatch -m 0700 /var/lib/croustypatch
COPY --from=build /src/target/release/croustypatch-server /usr/local/bin/croustypatch-server
# The Linux agent is served to new hosts by the "Add agent" installer. Binaries for other platforms (FreeBSD must be
# built on FreeBSD) can be dropped into /var/lib/croustypatch/agents, which lives in the persisted data volume.
COPY --from=build /out/ /usr/share/croustypatch/agents/
COPY --from=web /web/out /usr/share/croustypatch/web
ENV CROUSTYPATCH_STATIC_DIR=/usr/share/croustypatch/web \
    CROUSTYPATCH_DATA_DIR=/var/lib/croustypatch \
    CROUSTYPATCH_AGENT_DIR=/usr/share/croustypatch/agents,/var/lib/croustypatch/agents \
    CROUSTYPATCH_LISTEN=0.0.0.0:8443
USER croustypatch
EXPOSE 8443
VOLUME ["/var/lib/croustypatch"]
# No HEALTHCHECK: the slim image has no HTTP client. Probe GET /api/v1/health from your orchestrator.
ENTRYPOINT ["/usr/local/bin/croustypatch-server"]
CMD ["serve"]
