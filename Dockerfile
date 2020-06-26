# ------------------------------------------------------------------------------
# Cargo Build Stage
# ------------------------------------------------------------------------------

FROM rust:latest as cargo-build

ARG FEATURES

RUN apt-get update

RUN apt-get install musl-tools -y pkg-config libssl-dev

RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /usr/src/kalaxia-api

COPY Cargo.toml Cargo.lock /usr/src/kalaxia-api/

RUN mkdir src/

RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > src/main.rs

RUN RUSTFLAGS=-Clinker=musl-gcc cargo build --release --target=x86_64-unknown-linux-musl --features=$FEATURES

RUN rm -f target/x86_64-unknown-linux-musl/release/deps/kalaxia_api*

COPY src src/

RUN RUSTFLAGS=-Clinker=musl-gcc cargo build --release --target=x86_64-unknown-linux-musl --features=$FEATURES

# ------------------------------------------------------------------------------
# Final Stage
# ------------------------------------------------------------------------------

FROM alpine:latest

RUN addgroup -g 1000 kalaxia

RUN adduser -D -s /bin/sh -u 1000 -G kalaxia kalaxia

WORKDIR /home/kalaxia/bin/

RUN apk add --no-cache ca-certificates

COPY --from=builder /out/diesel /bin/

COPY --from=cargo-build /usr/src/kalaxia-api/target/x86_64-unknown-linux-musl/release/kalaxia-api .

RUN chown kalaxia:kalaxia kalaxia-api

USER kalaxia

CMD ["./kalaxia-api"]
