MATCH (us:UserStory {id: $id})-[:HAS_HISTORY]->(h:EditHistory)
RETURN h.id AS id,
       h.timestamp AS timestamp,
       h.userName AS userName,
       h.userEmail AS userEmail,
       h.changes AS changes
ORDER BY h.timestamp DESC
LIMIT 50
