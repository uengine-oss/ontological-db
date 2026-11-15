MATCH (bc:BoundedContext {id: $bc_id})
MERGE (j:Journey {key: $journey_key})
  ON CREATE SET j.id = $journey_id, j.createdAt = datetime()
SET j.journeyId = $journey_slug,
    j.name = $name,
    j.description = $description,
    j.boundedContextId = $bc_id,
    j.source = $source,
    j.updatedAt = datetime()
MERGE (bc)-[:HAS_JOURNEY]->(j)
RETURN j.id AS id
