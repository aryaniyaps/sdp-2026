# One image for the hosted setup: the memory service (its extraction worker shells out to Pi) and
# ttyd running Pi with the memory extension. Built on the VM by deploy.sh, whose Docker has no
# BuildKit, so there are no cache mounts here.
FROM rust:1.96-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY migrations migrations
COPY src src
RUN cargo build --release --locked && cp target/release/memory-engine /memory-engine

FROM node:22-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl tini less ripgrep fd-find \
    && rm -rf /var/lib/apt/lists/*
# ttyd is not packaged for Debian 12: pinned release, checksum verified.
RUN curl -fsSL -o /usr/local/bin/ttyd https://github.com/tsl0922/ttyd/releases/download/1.7.7/ttyd.x86_64 \
    && echo "8a217c968aba172e0dbf3f34447218dc015bc4d5e59bf51db2f2cd12b7be4f55  /usr/local/bin/ttyd" | sha256sum -c - \
    && chmod +x /usr/local/bin/ttyd
# The version the extension is tested against (integrations/pi/package.json pins ^1.0.2).
RUN npm install -g @earendil-works/pi-coding-agent@1.0.2 && npm cache clean --force
RUN ln -s "$(command -v fdfind)" /usr/local/bin/fd
COPY integrations/pi /opt/memory-pi
RUN cd /opt/memory-pi && npm ci --legacy-peer-deps --omit=dev --ignore-scripts || npm install --legacy-peer-deps --omit=dev --ignore-scripts
COPY --from=build /memory-engine /usr/local/bin/memory-engine
COPY deploy/azure/pi-session.sh /usr/local/bin/pi-session
RUN chmod +x /usr/local/bin/pi-session \
    && useradd --create-home --uid 10001 --shell /bin/bash pi \
    && mkdir -p /work /tmp/pi-sessions && chown -R pi:pi /work /tmp/pi-sessions /opt/memory-pi
USER pi
ENV HOME=/home/pi PI_OFFLINE=1
WORKDIR /work
ENTRYPOINT ["/usr/bin/tini", "--"]
