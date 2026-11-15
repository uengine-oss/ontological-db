CREATE (d:DeletionRecord {
    batchId: $batch_id, scope: $scope,
    rootLabel: $root_label, rootName: $root_name,
    actor: $actor, createdAt: $created_at, restored: false,
    nodeCount: $node_count, relCount: $rel_count, snapshot: $snapshot
})
