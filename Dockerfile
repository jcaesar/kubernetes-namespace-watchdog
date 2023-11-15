FROM nixos/nix:latest AS builder

COPY . /tmp/build
WORKDIR /tmp/build

RUN nix \
    --extra-experimental-features "nix-command flakes" \
    --option filter-syscalls false \
    build

#RUN --mount=type=cache,target=/nix/cache \
#	set -eu; \
#	for f in /nix/store/*; do ln -sf /nix/store/$f /nix/cache/$(basename $f); done; \
#	nix \
#		--store /nix/cache \
#    --extra-experimental-features "nix-command flakes" \
#    --option filter-syscalls false \
#    build

RUN mkdir /tmp/nix-store-closure
RUN nix-store -qR result/ | xargs cp -R -t/tmp/nix-store-closure

FROM scratch

WORKDIR /app
COPY --from=builder /tmp/nix-store-closure /nix/store
COPY --from=builder /tmp/build/result/bin/kubernetes-namespace-watchdog-watcher /app/
ENTRYPOINT ["/app/kubernetes-namespace-watchdog-watcher"]
