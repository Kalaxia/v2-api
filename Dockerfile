FROM rust:1.43 as builder
ARG FEATURES
WORKDIR /usr/src/kalaxia-api
COPY . .
RUN cargo install --path . --features=$FEATURES

FROM debian:buster-slim
RUN apt-get update && apt-get install -y pkg-config libssl-dev
COPY --from=builder /usr/local/cargo/bin/kalaxia-api /usr/local/bin/kalaxia-api
CMD ["kalaxia-api"]