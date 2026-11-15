MATCH (d:DeletionRecord {batchId: $b}) SET d.restored = true, d.restoredAt = $ts
