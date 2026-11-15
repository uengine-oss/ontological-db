MATCH (t:BpmTask {id: $tid})
OPTIONAL MATCH (t)-[:PROMOTED_TO]->(:UserStory)-[:IMPLEMENTS]->(c1:Command)
OPTIONAL MATCH (c2:Command)-[:PROMOTED_FROM]->(t)
WITH collect(DISTINCT c1.id) + collect(DISTINCT c2.id) AS ids
UNWIND ids AS cid
WITH DISTINCT cid WHERE cid IS NOT NULL
RETURN cid
