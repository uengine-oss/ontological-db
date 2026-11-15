CREATE (us:UserStory {
    id: $us_id,
    role: $role,
    action: $action,
    benefit: $benefit,
    priority: 'medium',
    status: 'new',
    acceptanceCriteria: $acceptance_criteria,
    createdAt: datetime()
})
RETURN us.id as id
