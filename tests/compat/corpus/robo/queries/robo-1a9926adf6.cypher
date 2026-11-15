UNWIND $ids AS uid
OPTIONAL MATCH (u:UI {id: uid})<-[*1..30]-(c:Command)
WHERE c IS NOT NULL AND NOT EXISTS { (:Policy)-[:INVOKES]->(c) }
WITH uid, c LIMIT 1
OPTIONAL MATCH (m:StoryboardPageMapping {commandId: c.id})
RETURN uid AS id,
       coalesce(m.status, 'active') = 'archived' AS archived
