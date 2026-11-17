# Supabase Postgres + Apache AGE (unmodified upstream AGE, three files added).
#
# Build with ./build.sh, which first compiles AGE in a throwaway
# postgres:<minor>-bookworm container and then passes the artifacts to this
# file as the named build context "age-artifacts":
#
#   docker build --build-context age-artifacts=<dir with age.so, age.control, age--*.sql> \
#                -t uengine/supabase-postgres-age:15.8.1.060-age1.8.0 .
#
# Only one RUN layer is added on top of the Supabase image. The artifacts are
# bind-mounted (not COPYed), so nothing from the build stays in the image.
ARG SUPABASE_POSTGRES_TAG=15.8.1.060
FROM supabase/postgres:${SUPABASE_POSTGRES_TAG}

ARG SUPABASE_POSTGRES_TAG
ARG AGE_VERSION=1.8.0
ARG AGE_REF=PG15/v1.8.0-rc0
LABEL org.opencontainers.image.title="supabase-postgres-age" \
      org.opencontainers.image.description="supabase/postgres with Apache AGE preloaded" \
      org.opencontainers.image.base.name="supabase/postgres:${SUPABASE_POSTGRES_TAG}" \
      org.uengine.age.version="${AGE_VERSION}" \
      org.uengine.age.ref="${AGE_REF}"

USER root
RUN --mount=type=bind,from=age-artifacts,target=/tmp/age \
    --mount=type=bind,source=sql,target=/tmp/age-sql \
    set -eu; \
    # 1) the server's real library directory: NIX_PGLIBDIR in the wrapper script
    #    (pg_config --pkglibdir points at a different store path)
    wrapper="$(readlink -f /usr/bin/postgres)"; \
    libdir="$(sed -n "s/^export NIX_PGLIBDIR='\(.*\)'.*/\1/p" "$wrapper")"; \
    test -d "$libdir"; \
    install -m 755 /tmp/age/age.so "$libdir/age.so"; \
    # 2) extension control/SQL: the share dir pg_config reports (the one the
    #    running server reads, checked by moving the file away) and, to be safe
    #    if a later image resolves it from the binary's path, the one next to libdir
    for extdir in "$(dirname "$libdir")/share/postgresql/extension" \
                  "$(readlink -f "$(pg_config --sharedir)/extension")"; do \
      test -d "$extdir"; \
      install -m 644 /tmp/age/age.control /tmp/age/age--*.sql "$extdir/"; \
      echo "age control/sql -> $extdir"; \
    done; \
    # 3) preload: append ", age" to the existing list in the image layer
    #    (/etc/postgresql-custom is a volume in Compose, so it is not used)
    conf=/etc/postgresql/postgresql.conf; \
    grep -q "^shared_preload_libraries = '[^']*'" "$conf"; \
    if ! grep -qE "^shared_preload_libraries = '[^']*\bage\b" "$conf"; then \
      sed -i -E "s/^(shared_preload_libraries = '[^']*)'/\1, age'/" "$conf"; \
    fi; \
    grep -n "^shared_preload_libraries" "$conf"; \
    # 4) fresh data volumes: install AGE as supabase_admin during initdb
    #    (runs after the stock migrations; existing volumes use scripts/install-age.sh)
    install -m 644 /tmp/age-sql/99999999999999_age.sql \
      /docker-entrypoint-initdb.d/migrations/99999999999999_age.sql; \
    cp /tmp/age/BUILDINFO /etc/postgresql/age-BUILDINFO 2>/dev/null || true

# the stock image's entrypoint/cmd are kept as they are
