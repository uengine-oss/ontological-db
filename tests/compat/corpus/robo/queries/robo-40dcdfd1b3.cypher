MATCH (f:Feature {id: $id})
SET f.name = $name, f.description = $description
RETURN f {.id, .key, .name, .description, .source, .boundedContextId} AS feature
