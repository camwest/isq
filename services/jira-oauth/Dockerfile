FROM rust:1.87-slim AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/jira-oauth /usr/local/bin/

ENV PORT=3000
EXPOSE 3000

CMD ["jira-oauth"]
