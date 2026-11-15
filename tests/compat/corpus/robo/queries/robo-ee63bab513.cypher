CREATE (n:Aggregate {
    id: $id, name: $name, title: $name, displayName: $name, description: $desc,
    key: $key, proposalSource: $pid, createdAt: datetime(), updatedAt: datetime()
})
