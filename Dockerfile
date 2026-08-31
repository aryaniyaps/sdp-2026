FROM rust:1.96-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/memory-engine /usr/local/bin/memory-engine
EXPOSE 8080
ENTRYPOINT ["memory-engine"]

