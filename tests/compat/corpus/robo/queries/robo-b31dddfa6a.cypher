MATCH (us:UserStory)
WHERE NOT (us)-[:IMPLEMENTS]->(:BoundedContext)
RETURN {
    id: us.id,
    role: us.role,
    action: us.action,
    benefit: us.benefit,
    priority: us.priority,
    status: us.status,
    acceptanceCriteria: coalesce(us.acceptanceCriteria, []),
    criteriaUserEdited: coalesce(us.criteriaUserEdited, false),
    criteriaEditedAt: us.criteriaEditedAt
} as userStory
ORDER BY us.createdAt DESC
