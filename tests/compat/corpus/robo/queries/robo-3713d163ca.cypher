CREATE (p:Proposal {
    id: $id,
    title: $title,
    originalPrompt: $originalPrompt,
    author: $author,
    createdAt: datetime($createdAt),
    status: 'DRAFT',
    statusHistory: '[]',
    clarificationLog: '[]',
    decompositionMode: $decompositionMode
})
