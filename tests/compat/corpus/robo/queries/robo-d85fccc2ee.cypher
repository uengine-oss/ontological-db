MATCH (n)-[:HAS_HISTORY]->(h:EditHistory)
WHERE n.id IN $ids
RETURN n.id AS itemId, labels(n) AS labels,
       CASE WHEN 'UserStory' IN labels(n)
            THEN coalesce(n.role, '') + ': ' + coalesce(n.action, '')
            ELSE coalesce(n.displayName, n.name) END AS itemName,
       h.id AS id, h.timestamp AS timestamp,
       h.userName AS userName, h.userEmail AS userEmail,
       h.changes AS changes, h.source AS source,
       h.feedback AS feedback, h.rationale AS rationale
ORDER BY h.timestamp DESC
LIMIT $limit
