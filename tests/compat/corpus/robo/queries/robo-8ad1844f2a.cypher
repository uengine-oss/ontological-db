MATCH (us:UserStory)
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(bc:BoundedContext)
RETURN {
    id: us.id,
    role: us.role,
    action: us.action,
    benefit: us.benefit,
    priority: us.priority,
    status: us.status,
    bcId: bc.id,
    bcName: bc.name,
    acceptanceCriteria: coalesce(us.acceptanceCriteria, []),
    criteriaUserEdited: coalesce(us.criteriaUserEdited, false),
    criteriaEditedAt: us.criteriaEditedAt
} as user_story
ORDER BY us.id
