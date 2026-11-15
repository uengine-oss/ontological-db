MATCH (chg:RequirementChange {id: $id})-[e:EFFECT]->(n)
RETURN n.id AS nodeId,
       CASE WHEN 'CreationIntent' IN labels(n)
            THEN n.nodeLabel
            ELSE labels(n)[0]
       END AS nodeLabel,
       COALESCE(n.title, n.name, n.action, '') AS nodeTitle,
       COALESCE(n.acceptanceCriteria, n.description, n.body, '') AS currentContent,
       e.reason AS reason,
       e.impactLevel AS impactLevel,
       COALESCE(e.changeType, 'MODIFY') AS changeType,
       e.templateData AS templateData,
       e.appliedNodeId AS appliedNodeId
