MATCH (chg:RequirementChange {id: $id})-[e:EFFECT]->(n)
WHERE e.diff IS NOT NULL
RETURN n.id AS nodeId,
       CASE WHEN 'CreationIntent' IN labels(n)
            THEN n.nodeLabel
            ELSE labels(n)[0]
       END AS nodeLabel,
       COALESCE(n.title, n.name, n.action, '') AS nodeTitle,
       e.diff AS diff,
       e.impactLevel AS impactLevel,
       e.appliedAt AS appliedAt,
       e.templateData AS templateData,
       e.appliedNodeId AS appliedNodeId
