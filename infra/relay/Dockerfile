FROM rust:1-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /build/target/release/teamscord-relay /usr/local/bin/teamscord-relay
RUN mkdir -p /app/data
ENV TEAMSCORD_RELAY_IDENTITY_PATH=/app/data/identity.bin
EXPOSE 4001/tcp 4002/udp
VOLUME ["/app/data"]
ENTRYPOINT ["/usr/local/bin/teamscord-relay"]
