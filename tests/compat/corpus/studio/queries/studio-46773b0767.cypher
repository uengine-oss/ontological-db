MATCH (d:Document {document_id: $document_id})-[:HAS_CHUNK]->(:Chunk)-[r:NEXT_CHUNK]->(:Chunk)
DELETE r
