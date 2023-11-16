# Slimmed version of https://github.com/jcaesar/kafka-totop/blob/f9559bd8361895775759f6781dbd83138a25a879/rel-cross.Dockerfile

FROM --platform=$BUILDPLATFORM rust:1.72.0-bookworm AS builder

RUN for a in armhf arm64 amd64; do dpkg --add-architecture $a; done \
  && apt update \
  && apt-get -y install eatmydata \
  && eatmydata apt install -y \
    build-essential \
    clang \
    cmake \
    crossbuild-essential-amd64 \
    crossbuild-essential-arm64 \
    lld \
  && true  
RUN rustup target add aarch64-unknown-linux-musl x86_64-unknown-linux-musl
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=/usr/bin/clang
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-arg=--ld-path=/usr/bin/ld.lld -C link-arg=--target=aarch64-unknown-linux-musl"
ENV CARGO_TARGET_X86_64_UNKOWN_LINUX_MUSL_LINKER=/usr/bin/clang
ENV CARGO_TARGET_X86_64_UNKOWN_LINUX_MUSL_RUSTFLAGS="-C link-arg=--ld-path=/usr/bin/ld.lld -C link-arg=--target=x86_64-unknown-linux-musl"
ENV CC=/usr/bin/clang
ENV CXX=/usr/bin/true
ENV LDFLAGS=--ld-path=/usr/bin/ld.lld
WORKDIR /usr/src/project

FROM builder as src
COPY . .
RUN echo >owner/src/main.rs

FROM builder as build
COPY --from=src /usr/src/project /usr/src/project
ARG TARGETPLATFORM
RUN case $TARGETPLATFORM in \
    linux/amd64) t=x86_64;; \
    linux/arm64) t=aarch64;; \
    *) echo Unknown target platfrom $TARGETPLATFORM; exit 1;; \
  esac; \
  cargo build --locked --target $t-unknown-linux-musl --profile maxopt -p kubernetes-namespace-watchdog-watcher && \
  ln -s $t-unknown-linux-musl target/cross
RUN file target/cross/maxopt/kubernetes-namespace-watchdog-watcher | tee /dev/stderr | grep -qE 'static(ally|-pie) linked'

FROM alpine as final
COPY --from=build /usr/src/project/target/cross/maxopt/kubernetes-namespace-watchdog-watcher /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/kubernetes-namespace-watchdog-watcher"]
