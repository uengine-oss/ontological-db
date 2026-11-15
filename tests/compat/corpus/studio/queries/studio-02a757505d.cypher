MATCH (n)
WHERE elementId(n) IN $target_node_ids
RETURN
  n.document_id AS document_id,
  n.chunk_ref AS chunk_ref,
  n.source_page AS source_page
