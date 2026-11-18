#!/usr/bin/env bash
# Build the Bolt gateway image with the Apache AGE backend: uengine/ontological-bolt:age.
#
#   ./gateway/build.sh
#
# 1. The gateway (bolt/ of this repository, with cypher-syntax/ and the engine's
#    parser files it includes) is compiled in a throwaway rust:1-alpine container
#    into a static musl binary. The source is mounted read-only; crates are kept
#    in the named volume agesb-cargo-registry, the build in $TARGET_DIR.
# 2. The binary goes into an alpine image (one COPY), user nobody, port 7687,
#    OG_BOLT_BACKEND=age.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
IMAGE="${IMAGE:-uengine/ontological-bolt:age}"
RUST_IMAGE="${RUST_IMAGE:-rust:1-alpine}"
BASE_IMAGE="${BASE_IMAGE:-alpine:3.20}"
TARGET_DIR="${TARGET_DIR:-$here/target}"   # ignored by the repository's target/ rule
REGISTRY_VOLUME="${REGISTRY_VOLUME:-agesb-cargo-registry}"
BUILD_MEMORY="${BUILD_MEMORY:-1500m}"
BUILD_CPUS="${BUILD_CPUS:-2}"

mkdir -p "$TARGET_DIR"
echo "== 1/2 compile bolt/ ($RUST_IMAGE, static musl)"
t0=$(date +%s)
docker run --rm --name agesb-bolt-build --memory "$BUILD_MEMORY" --cpus "$BUILD_CPUS" \
  -v "$repo:/src:ro" -v "$TARGET_DIR:/target" -v "$REGISTRY_VOLUME:/usr/local/cargo/registry" \
  -e CARGO_TARGET_DIR=/target -w /src/bolt "$RUST_IMAGE" \
  sh -c 'apk add --no-cache musl-dev >/dev/null 2>&1 || true; cargo build --release --locked && cargo test --release --locked 2>&1 | tail -3'
t1=$(date +%s)
bin="$TARGET_DIR/release/ontological-bolt"
test -x "$bin"

echo "== 2/2 docker build $IMAGE"
ctx="$(mktemp -d "${TMPDIR:-/tmp}/age-bolt-ctx.XXXXXX")"
trap 'rm -rf "$ctx"' EXIT
cp "$bin" "$ctx/ontological-bolt"
rev="$(git -C "$repo" rev-parse --short HEAD 2>/dev/null || echo unknown)"
docker build -q -f "$here/Dockerfile" --build-arg BASE_IMAGE="$BASE_IMAGE" --build-arg REVISION="$rev" -t "$IMAGE" "$ctx"
docker tag "$IMAGE" "${IMAGE%%:*}:age-$rev"
t2=$(date +%s)
echo "== done: $IMAGE (also ${IMAGE%%:*}:age-$rev); compile $((t1 - t0)) s, image $((t2 - t1)) s"
docker image inspect "$IMAGE" --format 'arch={{.Architecture}} size={{.Size}}'
