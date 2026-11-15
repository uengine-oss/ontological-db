MATCH (bc:BoundedContext)-[:HAS_JOURNEY]->(j:Journey)
WHERE bc.id IN $bc_ids
MATCH (j)-[:HAS_STEP]->(s1:JourneyStep)-[n:NEXT]->(s2:JourneyStep)
MATCH (j)-[:HAS_STEP]->(s2)
OPTIONAL MATCH (s1)-[:SHOWS]->(u1:UI)
OPTIONAL MATCH (s2)-[:SHOWS]->(u2:UI)
RETURN n.id AS id,
       s1.kind AS s1_kind, s1.id AS s1_id, u1.id AS u1_id,
       s2.kind AS s2_kind, s2.id AS s2_id, u2.id AS u2_id,
       coalesce(n.condition, '') AS condition,
       coalesce(n.source, 'llm') AS source,
       coalesce(n.documentExcerpt, '') AS document_excerpt,
       coalesce(j.journeyId, '') AS journey_id,
       coalesce(j.name, '') AS journey_name,
       toString(n.createdAt) AS created_at,
       toString(n.updatedAt) AS updated_at
ORDER BY n.createdAt
