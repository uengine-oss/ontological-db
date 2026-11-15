MATCH (d:DeletionRecord {batchId: $b}) DELETE d RETURN count(d) AS c
