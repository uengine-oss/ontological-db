# The Bolt gateway with the Apache AGE backend (bolt/ of this repository).
# Built by gateway/build.sh, which compiles a static musl binary first and
# passes it as the only file of the build context.
ARG BASE_IMAGE=alpine:3.20
FROM ${BASE_IMAGE}
ARG REVISION=unknown
LABEL org.opencontainers.image.title="ontological-bolt (AGE backend)" \
      org.opencontainers.image.description="Bolt (Neo4j protocol) gateway in front of Apache AGE" \
      org.opencontainers.image.revision="${REVISION}"
COPY ontological-bolt /usr/local/bin/ontological-bolt
USER nobody
ENV OG_BOLT_BACKEND=age \
    OG_BOLT_LISTEN=0.0.0.0:7687 \
    OG_BOLT_PGPORT=5432 \
    OG_BOLT_PGDATABASE=graph
EXPOSE 7687
HEALTHCHECK --interval=5s --timeout=3s --retries=20 CMD nc -z 127.0.0.1 7687 || exit 1
ENTRYPOINT ["/usr/local/bin/ontological-bolt"]
