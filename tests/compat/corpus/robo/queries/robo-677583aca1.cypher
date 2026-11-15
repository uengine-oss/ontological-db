CREATE (us:UserStory {
    id: $id, role: $role, action: $action, benefit: $benefit,
    name: $name, displayName: $name,
    acceptanceCriteria: $ac, status: 'draft', priority: 'medium',
    proposalSource: $pid, createdAt: datetime(), updatedAt: datetime()
})
