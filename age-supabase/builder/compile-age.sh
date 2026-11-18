#!/usr/bin/env bash
# Runs INSIDE a throwaway postgres:<PG_MINOR>-bookworm container (see ../build.sh).
# Compiles Apache AGE against the exact PostgreSQL minor version of the Supabase
# image and leaves only the three install artifacts in /out/age:
#   age.so, age.control, age--*.sql
#
# Inputs (env):
#   AGE_REF     git ref to build (default PG15/v1.8.0-rc0)
#   AGE_COMMIT  expected commit of AGE_REF (optional; build stops if it differs)
#   JOBS        make -j value (default 2)
#   PG_VERSION  set by the postgres image, e.g. 15.8-1.pgdg120+1
# Mounts:
#   /src  (optional, read-only) an existing AGE checkout; otherwise AGE_REF is cloned
#   /out  output directory (written)
set -euo pipefail

AGE_REF="${AGE_REF:-PG15/v1.8.0-rc0}"
JOBS="${JOBS:-2}"
PG_MAJOR="${PG_MAJOR:?PG_MAJOR not set (is this the postgres image?)}"
PG_VERSION="${PG_VERSION:?PG_VERSION not set (is this the postgres image?)}"
PG_CONFIG="/usr/lib/postgresql/${PG_MAJOR}/bin/pg_config"
export DEBIAN_FRONTEND=noninteractive

log() { printf '\n== %s\n' "$*"; }

log "apt sources: add the PGDG archive so the exact server-dev ${PG_VERSION} can be pinned"
. /etc/os-release
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates >/dev/null   # the archive is https only
keyring="$(sed -n 's/.*signed-by=\([^] ]*\).*/\1/p' /etc/apt/sources.list.d/pgdg.list | head -1)"
echo "deb [ signed-by=${keyring} ] https://apt-archive.postgresql.org/pub/repos/apt ${VERSION_CODENAME}-pgdg-archive main" \
  > /etc/apt/sources.list.d/pgdg-archive.list
apt-get update -qq

log "install build tools + postgresql-server-dev-${PG_MAJOR}=${PG_VERSION}"
if apt-get install -y -qq --no-install-recommends \
     build-essential flex bison perl git ca-certificates binutils \
     "postgresql-server-dev-${PG_MAJOR}=${PG_VERSION}" >/tmp/apt.log 2>&1; then
  HEADERS="pinned ${PG_VERSION}"
else
  tail -20 /tmp/apt.log
  echo "WARN: pinned server-dev ${PG_VERSION} not installable, falling back to the newest ${PG_MAJOR}.x" >&2
  apt-get install -y -qq --no-install-recommends \
     build-essential flex bison perl git ca-certificates binutils \
     "postgresql-server-dev-${PG_MAJOR}" >/tmp/apt.log 2>&1
  HEADERS="unpinned $(dpkg-query -W -f='${Version}' "postgresql-server-dev-${PG_MAJOR}")"
fi

log "source"
rm -rf /build && mkdir -p /build
if [ -d /src/.git ] || [ -f /src/age.control ]; then
  cp -a /src/. /build/
  SRC_FROM="mounted /src"
else
  git clone -q --depth 1 --branch "${AGE_REF}" https://github.com/apache/age.git /build
  SRC_FROM="git clone ${AGE_REF}"
fi
cd /build
commit="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
if [ -n "${AGE_COMMIT:-}" ] && [ "${commit#"${AGE_COMMIT}"}" = "${commit}" ]; then
  echo "ERROR: source is at ${commit}, expected ${AGE_COMMIT}" >&2
  exit 1
fi
# a mounted checkout may carry build products from elsewhere
make clean PG_CONFIG="${PG_CONFIG}" >/dev/null 2>&1 || true

log "compile (make -j${JOBS})"
make -j"${JOBS}" PG_CONFIG="${PG_CONFIG}" >/tmp/make.log 2>&1 || { tail -60 /tmp/make.log; exit 1; }
grep -iE 'warning|conflict' /tmp/make.log | head -20 || true

log "install into a staging root"
rm -rf /stage
make install DESTDIR=/stage PG_CONFIG="${PG_CONFIG}" >/tmp/install.log 2>&1 || { tail -40 /tmp/install.log; exit 1; }

libdir="$("${PG_CONFIG}" --pkglibdir)"
sharedir="$("${PG_CONFIG}" --sharedir)"
rm -rf /out/age && mkdir -p /out/age
install -m 755 "/stage${libdir}/age.so" /out/age/age.so
strip --strip-debug /out/age/age.so
install -m 644 "/stage${sharedir}/extension/age.control" /out/age/
install -m 644 "/stage${sharedir}/extension/"age--*.sql /out/age/

log "check: dynamic dependencies of age.so"
readelf -d /out/age/age.so | grep NEEDED | tee /tmp/needed.txt
# glibc only: libc, libm and (on aarch64) the dynamic loader itself
if grep -vE 'libc\.so\.6|libm\.so\.6|ld-linux-[a-z0-9_-]+\.so\.[0-9]' /tmp/needed.txt | grep -q NEEDED; then
  echo "ERROR: age.so links something other than glibc" >&2
  exit 1
fi
max_glibc="$(objdump -T /out/age/age.so | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"

{
  echo "age_ref=${AGE_REF}"
  echo "age_commit=${commit}"
  echo "source=${SRC_FROM}"
  echo "builder_os=${PRETTY_NAME}"
  echo "builder_glibc=$(ldd --version | head -1 | awk '{print $NF}')"
  echo "age_so_max_glibc_symbol=${max_glibc}"
  echo "server_headers=${HEADERS}"
  echo "pg_config=$("${PG_CONFIG}" --version)"
  echo "gcc=$(gcc -dumpfullversion)"
  echo "bison=$(bison --version | head -1)"
  echo "flex=$(flex --version | head -1)"
  echo "arch=$(uname -m)"
  echo "files:"
  (cd /out/age && sha256sum age.so age.control age--*.sql)
} | tee /out/age/BUILDINFO
