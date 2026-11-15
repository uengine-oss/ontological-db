MATCH (bc:BoundedContext)
OPTIONAL MATCH (us:UserStory)-[:IMPLEMENTS]->(bc)
WITH bc, us,
  CASE WHEN us IS NOT NULL AND NOT (us)-[:IMPLEMENTS]->(:Command) AND NOT (us)-[:IMPLEMENTS]->(:ReadModel) AND NOT (us)-[:IMPLEMENTS]->(:Event) AND NOT (us)-[:IMPLEMENTS]->(:Policy) THEN us ELSE NULL END AS orphan
WITH bc, count(DISTINCT us) AS total,
     [o IN collect(DISTINCT orphan) WHERE o IS NOT NULL] AS orphans
RETURN bc.id AS id, bc.name AS name, total AS totalUS,
       size(orphans) AS orphanUS, [u IN orphans[0..5] | u.action] AS sample
ORDER BY orphanUS DESC
