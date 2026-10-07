FROM node:22-bookworm-slim AS frontend
WORKDIR /ui
COPY frontend/package*.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM rust:1.96-bookworm AS builder
WORKDIR /app
COPY . .
COPY --from=frontend /ui/dist ./frontend/dist
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/memory-engine /usr/local/bin/memory-engine
EXPOSE 8080
ENTRYPOINT ["memory-engine"]

