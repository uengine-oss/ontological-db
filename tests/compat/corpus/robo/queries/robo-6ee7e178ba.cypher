MATCH (chg:RequirementChange {id: $change_id})
CREATE (ci:CreationIntent {
    id: $placeholder_id,
    nodeLabel: $node_label,
    templateData: $template_json,
    reason: $reason,
    createdAt: datetime()
})
CREATE (chg)-[e:EFFECT]->(ci)
SET e.reason = $reason,
    e.impactLevel = $impact_level,
    e.changeType = 'CREATE',
    e.templateData = $template_json
