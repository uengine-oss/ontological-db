MATCH (chg:RequirementChange {id: $id})-[e:EFFECT]->(t)
RETURN t.id AS nodeId,
       CASE WHEN 'CreationIntent' IN labels(t)
            THEN t.nodeLabel
            ELSE labels(t)[0]
       END AS nodeLabel,
       CASE WHEN 'CreationIntent' IN labels(t)
            THEN t.templateData
            ELSE COALESCE(t.title, t.name, t.action, '')
       END AS nodeTitle,
       e.reason AS reason,
       e.impactLevel AS impactLevel,
       COALESCE(e.changeType, 'MODIFY') AS changeType,
       e.templateData AS templateData,
       e.appliedNodeId AS appliedNodeId
