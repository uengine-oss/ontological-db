MATCH (f:Feature {id: $id})
RETURN f {.id, .key, .name, .description, .source, .boundedContextId} AS feature
