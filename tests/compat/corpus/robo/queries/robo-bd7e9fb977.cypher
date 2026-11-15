MATCH (bc:BoundedContext {id: $bc_id})
    MERGE (f:Feature {key: $key})
      ON CREATE SET f.id = randomUUID(),
                    f.createdAt = datetime(),
                    f.source = $source,
                    f.session_id = $session_id
    SET f.name = $name,
        f.description = $description,
        f.boundedContextId = bc.id,
        f.sequence = $sequence,
        f.updatedAt = datetime(),
f.edgeCases = $edgeCases
    MERGE (bc)-[hf:HAS_FEATURE]->(f)
      ON CREATE SET hf.createdAt = datetime()
    RETURN f {.id, .key, .name, .description, .source, .boundedContextId,
               .edgeCases, .assumptions} AS feature
