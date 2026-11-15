MATCH (d:Document {document_id: $document_id})
OPTIONAL MATCH (d)-[:HAS_CHUNK]->(existing:Chunk)
WITH d, collect(existing.chunk_id) AS existing_ids
UNWIND $chunks AS chunk
MERGE (c:Chunk {chunk_id: chunk.chunk_id})
ON CREATE SET c.created_at = datetime()
SET c.document_id = chunk.document_id,
    c.document_name = chunk.document_name,
    c.chunk_ref = chunk.chunk_ref,
    c.source_text = chunk.source_text,
    c.source_page = chunk.source_page,
    c.chunk_index = chunk.chunk_index,
    c.char_count = chunk.char_count,
    c.text_hash = chunk.text_hash,
    c.page_text_hash = chunk.page_text_hash,
    c.page_sha256 = chunk.page_sha256,
    c.embedding = chunk.embedding,
    c.updated_at = datetime()
MERGE (d)-[:HAS_CHUNK]->(c)
