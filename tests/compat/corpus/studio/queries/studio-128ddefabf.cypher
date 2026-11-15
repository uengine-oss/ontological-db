CALL db.index.vector.queryNodes('chunk_embedding_vector', $candidate_limit, $embedding)
YIELD node, score
WHERE (size($document_ids) = 0 OR node.document_id IN $document_ids)
  AND (size($chunk_refs) = 0 OR node.chunk_ref IN $chunk_refs)
  AND (size($source_pages) = 0 OR node.source_page IN $source_pages)
RETURN node, score
LIMIT $candidate_limit
