MATCH (d:DeletionRecord)
RETURN d.batchId AS batchId, d.scope AS scope, d.rootLabel AS rootLabel,
       d.rootName AS rootName, d.actor AS actor, d.createdAt AS createdAt,
       d.restored AS restored, d.nodeCount AS nodeCount, d.relCount AS relCount
ORDER BY d.createdAt DESC
