MATCH (us:UserStory)-[:IMPLEMENTS]->(bc:BoundedContext {id: $bc_id})
RETURN {
    id: us.id,
    role: us.role,
    action: us.action,
    benefit: us.benefit,
    priority: us.priority,
    status: us.status,
    acceptanceCriteria: coalesce(us.acceptanceCriteria, []),
    criteriaUserEdited: coalesce(us.criteriaUserEdited, false)
} as user_story
ORDER BY us.id
