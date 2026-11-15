CREATE (n:RequirementChange {
    id: $id,
    title: $title,
    originalPrompt: $originalPrompt,
    author: $author,
    createdAt: datetime($createdAt),
    status: 'DRAFT',
    statusHistory: '[]',
    sourceType: $sourceType,
    changeSetId: null
})
RETURN n {.*} AS change
