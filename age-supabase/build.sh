#!/usr/bin/env bash
# Build uengine/supabase-postgres-age:<supabase tag>-age<age version>.
#
#   ./build.sh                      # arm64 on Apple Silicon, amd64 on an amd64 host
#   PLATFORM=linux/amd64 ./build.sh # cross build through emulation (slow, see README)
#
# Step 1 compiles AGE in a throwaway container (--rm, so no build cache or
# intermediate image is left behind). Step 2 adds the three files to the
# Supabase image with a single RUN layer.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"

SUPABASE_POSTGRES_TAG="${SUPABASE_POSTGRES_TAG:-15.8.1.060}"
PG_MINOR="${PG_MINOR:-15.8}"                 # must match the PostgreSQL inside the Supabase image
AGE_REF="${AGE_REF:-PG15/v1.8.0-rc0}"
AGE_COMMIT="${AGE_COMMIT:-88646eb}"          # commit of AGE_REF, checked by the builder
AGE_VERSION="${AGE_VERSION:-1.8.0}"
AGE_SRC="${AGE_SRC:-}"                       # optional local AGE checkout (read-only); empty = clone
AGE_ARTIFACTS="${AGE_ARTIFACTS:-}"           # optional dir with an earlier compile (age.so, age.control, age--*.sql,
                                             # BUILDINFO): skip step 1 and only rebuild the image
PLATFORM="${PLATFORM:-}"                     # empty = the Docker host's native platform
JOBS="${JOBS:-2}"
BUILD_MEMORY="${BUILD_MEMORY:-1500m}"
BUILD_CPUS="${BUILD_CPUS:-2}"
IMAGE="${IMAGE:-uengine/supabase-postgres-age:${SUPABASE_POSTGRES_TAG}-age${AGE_VERSION}}"
OUT_DIR="${OUT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/age-artifacts.XXXXXX")}"
KEEP_OUT="${KEEP_OUT:-0}"
BUILDER_NAME="${BUILDER_NAME:-agesb-build}"

platform_args=()
[ -n "$PLATFORM" ] && platform_args=(--platform "$PLATFORM")

mkdir -p "$OUT_DIR"
cleanup() { rc=$?; [ "$KEEP_OUT" = 1 ] || rm -rf "$OUT_DIR"; exit $rc; }
trap cleanup EXIT

if [ -n "$AGE_ARTIFACTS" ]; then
  echo "== 1/2 reuse the AGE build in ${AGE_ARTIFACTS}"
  for f in age.so age.control; do test -s "$AGE_ARTIFACTS/$f" || { echo "missing $AGE_ARTIFACTS/$f"; exit 1; }; done
  if [ -s "$AGE_ARTIFACTS/BUILDINFO" ]; then
    # the hashes recorded at compile time must still match
    (cd "$AGE_ARTIFACTS" && sed -n '/^files:/,$p' BUILDINFO | tail -n +2 | shasum -a 256 -c --quiet -) || { echo "artifacts differ from their BUILDINFO"; exit 1; }
    grep -q "^age_commit=${AGE_COMMIT}" "$AGE_ARTIFACTS/BUILDINFO" || { echo "BUILDINFO is not for ${AGE_COMMIT}"; exit 1; }
  fi
  mkdir -p "$OUT_DIR/age"; cp "$AGE_ARTIFACTS"/age.so "$AGE_ARTIFACTS"/age.control "$AGE_ARTIFACTS"/age--*.sql "$OUT_DIR/age/"
  [ -s "$AGE_ARTIFACTS/BUILDINFO" ] && cp "$AGE_ARTIFACTS/BUILDINFO" "$OUT_DIR/age/"
  t0=$(date +%s); t1=$t0
else
echo "== 1/2 compile AGE ${AGE_REF} against PostgreSQL ${PG_MINOR} (container ${BUILDER_NAME})"
src_mount=()
if [ -n "$AGE_SRC" ]; then
  src_mount=(-v "$(cd "$AGE_SRC" && pwd):/src:ro")
fi
t0=$(date +%s)
docker run --rm --name "$BUILDER_NAME" ${platform_args[@]+"${platform_args[@]}"} \
  --memory "$BUILD_MEMORY" --cpus "$BUILD_CPUS" \
  -e AGE_REF="$AGE_REF" -e AGE_COMMIT="$AGE_COMMIT" -e JOBS="$JOBS" \
  ${src_mount[@]+"${src_mount[@]}"} \
  -v "$OUT_DIR:/out" \
  -v "$here/builder/compile-age.sh:/compile-age.sh:ro" \
  --entrypoint bash "postgres:${PG_MINOR}-bookworm" /compile-age.sh
t1=$(date +%s)
fi

echo "== 2/2 docker build ${IMAGE}"
docker build ${platform_args[@]+"${platform_args[@]}"} \
  --build-context age-artifacts="$OUT_DIR/age" \
  --build-arg SUPABASE_POSTGRES_TAG="$SUPABASE_POSTGRES_TAG" \
  --build-arg AGE_VERSION="$AGE_VERSION" \
  --build-arg AGE_REF="$AGE_REF" \
  -t "$IMAGE" "$here"
t2=$(date +%s)

echo "== done: ${IMAGE}"
echo "compile: $((t1 - t0)) s, image: $((t2 - t1)) s"
docker image inspect "$IMAGE" --format 'arch={{.Architecture}} size={{.Size}} id={{.Id}}'
[ "$KEEP_OUT" = 1 ] && echo "artifacts kept in $OUT_DIR"
exit 0
