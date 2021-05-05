# ------------------------------------------------------------------------------
# Cargo Build Stage
# ------------------------------------------------------------------------------

FROM rust:1.44 as cargo-build

ARG FEATURES

RUN apt-get update

WORKDIR /usr/src/kalaxia-api

COPY Cargo.toml Cargo.lock /usr/src/kalaxia-api/

RUN mkdir src/

RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > src/main.rs

RUN cargo build --release --features=vendored,$FEATURES

RUN rm -f target/x86_64-unknown-linux-gnu/release/deps/kalaxia_api*

COPY src src/

RUN cargo build --release --features=vendored,$FEATURES

# ------------------------------------------------------------------------------
# Final Stage
# ------------------------------------------------------------------------------

FROM ubuntu:latest

WORKDIR /home/kalaxia/bin/

COPY --from=cargo-build /usr/src/kalaxia-api/target/x86_64-unknown-linux-gnu/release/kalaxia-api .

RUN apk add --no-cache ca-certificates libcap && \
    setcap 'cap_net_bind_service=+ep' /home/kalaxia/bin/kalaxia-api && \
    addgroup -g 1000 kalaxia && \
    adduser -D -s /bin/sh -u 1000 -G kalaxia kalaxia

CMD ["./kalaxia-api"]
