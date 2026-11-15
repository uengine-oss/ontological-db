MATCH (d:DeletionRecord {batchId: $b}) RETURN d.snapshot AS snapshot, d.restored AS restored
