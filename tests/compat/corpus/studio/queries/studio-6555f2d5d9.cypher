MATCH (d:Document {document_id: $document_id})-[old_rel:HAS_CHUNK]->(stale:Chunk)
WHERE NOT stale.chunk_id IN $chunk_ids
DELETE old_rel
DETACH DELETE stale
