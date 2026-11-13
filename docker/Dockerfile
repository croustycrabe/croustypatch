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

# Desktop app center (GPUI, a native window) for laptops and workstations. Its own stage so that a server-only change reuses
# the cache; built on bookworm so that the binary runs on Debian 12+, Ubuntu and Arch. It links libxcb and libxkbcommon and
# loads the Wayland, Vulkan and fontconfig libraries at run time.
FROM rust:1-bookworm AS gui
RUN apt-get update \
 && apt-get install -y --no-install-recommends clang cmake pkg-config libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libwayland-dev libfontconfig1-dev libfreetype6-dev libssl-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /gui
COPY portal-gui ./
RUN cargo build --release --locked \
 && mkdir -p /out \
 && cp target/release/croustypatch-portal-gui /out/croustypatch-portal-gui-linux-$(uname -m) \
 && cp icons/256x256.png /out/croustypatch-portal-gui.png \
 && echo "runtime libraries:" && ldd target/release/croustypatch-portal-gui | awk '{print "  " $1}'

# SCAP Security Guide content (OpenSCAP) handed to the agents: Debian and Ubuntu datastreams of a pinned release.
FROM python:3.13-slim-bookworm AS scap
ARG SSG_VERSION=0.1.82
ARG SSG_SHA256=765e84bdce7f9055f9b9c2dd0ee2b713d4255f8eec94eac6d35ea4973c28919c
COPY docker/scap-content.py /scap-content.py
RUN python3 /scap-content.py "$SSG_VERSION" "$SSG_SHA256" /out

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
COPY --from=gui /out/ /usr/share/croustypatch/agents/
COPY --from=web /web/out /usr/share/croustypatch/web
COPY --from=scap /out/ /usr/share/croustypatch/scap-content/
ENV CROUSTYPATCH_STATIC_DIR=/usr/share/croustypatch/web \
    CROUSTYPATCH_DATA_DIR=/var/lib/croustypatch \
    CROUSTYPATCH_AGENT_DIR=/usr/share/croustypatch/agents,/var/lib/croustypatch/agents \
    CROUSTYPATCH_SCAP_DIR=/usr/share/croustypatch/scap-content \
    CROUSTYPATCH_LISTEN=0.0.0.0:8443
USER croustypatch
EXPOSE 8443
VOLUME ["/var/lib/croustypatch"]
# No HEALTHCHECK: the slim image has no HTTP client. Probe GET /api/v1/health from your orchestrator.
ENTRYPOINT ["/usr/local/bin/croustypatch-server"]
CMD ["serve"]
