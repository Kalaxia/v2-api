# ------------------------------------------------------------------------------
# Cargo Build Stage
# ------------------------------------------------------------------------------

FROM rust:1.52.1 as cargo-build

ARG FEATURES

RUN apt-get update

WORKDIR /usr/src/kalaxia-api

COPY Cargo.toml Cargo.lock /usr/src/kalaxia-api/

RUN mkdir src/

RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > src/main.rs

RUN cargo build --release --features="vendored,$FEATURES"

RUN rm -f target/release/deps/kalaxia_api*

COPY src src/

RUN cargo build --release --features="vendored,$FEATURES"

# ------------------------------------------------------------------------------
# Final Stage
# ------------------------------------------------------------------------------

FROM ubuntu:latest

WORKDIR /home/kalaxia/bin/

COPY --from=cargo-build /usr/src/kalaxia-api/target/release/kalaxia-api .

RUN apt-get update

RUN apt-get install ca-certificates libcap2-bin -y && \
    setcap 'cap_net_bind_service=+ep' /home/kalaxia/bin/kalaxia-api && \
    addgroup --gid 1000 kalaxia && \
    adduser --disabled-login --shell /bin/sh -u 1000 --gid 1000 kalaxia

CMD ["./kalaxia-api"]
