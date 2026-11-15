MATCH (bc:BoundedContext)-[:HAS_JOURNEY]->(j:Journey)
WHERE bc.id IN $bc_ids
MATCH (j)-[:HAS_STEP]->(g:JourneyStep {kind: 'gateway'})
RETURN g.id AS id, g.key AS key, g.label AS label,
       'exclusive' AS kind,
       j.boundedContextId AS bounded_context_id,
       coalesce(g.source, 'llm') AS source,
       coalesce(j.journeyId, '') AS journey_id,
       coalesce(j.name, '') AS journey_name,
       toString(g.createdAt) AS created_at,
       toString(g.updatedAt) AS updated_at
ORDER BY g.createdAt
