MATCH (d:Document) WHERE d.document_id IS NOT NULL DETACH DELETE d
