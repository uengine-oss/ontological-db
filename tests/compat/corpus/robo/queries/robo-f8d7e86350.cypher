MATCH (bc:BoundedContext {id: $id})
SET bc.name = $name, bc.description = $description
RETURN bc {.id, .key, .name, .displayName, .description} AS bounded_context
