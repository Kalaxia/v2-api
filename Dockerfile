# ------------------------------------------------------------------------------
# Cargo Build Stage
# ------------------------------------------------------------------------------

FROM rust:1.44 as cargo-build

ARG FEATURES

RUN apt-get update

RUN apt-get install -y musl-tools

RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /usr/src/kalaxia-api

COPY Cargo.toml Cargo.lock /usr/src/kalaxia-api/

RUN mkdir src/

RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > src/main.rs

RUN RUSTFLAGS=-Clinker=musl-gcc cargo build --release --target=x86_64-unknown-linux-musl --features=vendored,$FEATURES

RUN rm -f target/x86_64-unknown-linux-musl/release/deps/kalaxia_api*

COPY src src/

RUN RUSTFLAGS=-Clinker=musl-gcc cargo build --release --target=x86_64-unknown-linux-musl --features=vendored,$FEATURES

# ------------------------------------------------------------------------------
# Final Stage
# ------------------------------------------------------------------------------

FROM alpine:latest

ARG VERSION

WORKDIR /home/kalaxia/bin/

COPY --from=cargo-build /usr/src/kalaxia-api/target/x86_64-unknown-linux-musl/release/kalaxia-api .

RUN apk add --no-cache ca-certificates libcap && \
    setcap 'cap_net_bind_service=+ep' /home/kalaxia/bin/kalaxia-api && \
    addgroup -g 1000 kalaxia && \
    adduser -D -s /bin/sh -u 1000 -G kalaxia kalaxia 

ENV API_VERSION $VERSION

CMD ["./kalaxia-api"]
