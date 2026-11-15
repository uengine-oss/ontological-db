MATCH (bc:BoundedContext)-[:HAS_JOURNEY]->(j:Journey {source: 'llm'})
WHERE bc.id IN $bc_ids AND NOT j.id IN $keep_ids
OPTIONAL MATCH (j)-[:HAS_STEP]->(st:JourneyStep)
DETACH DELETE st, j
RETURN count(DISTINCT j) AS deleted
