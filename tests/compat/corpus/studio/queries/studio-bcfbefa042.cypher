UNWIND $links AS link
MATCH (d:Document {document_id: $document_id})-[:HAS_CHUNK]->(from_chunk:Chunk {chunk_id: link.from_chunk_id})
MATCH (d)-[:HAS_CHUNK]->(to_chunk:Chunk {chunk_id: link.to_chunk_id})
MERGE (from_chunk)-[r:NEXT_CHUNK]->(to_chunk)
ON CREATE SET r.created_at = datetime()
SET r.updated_at = datetime()
