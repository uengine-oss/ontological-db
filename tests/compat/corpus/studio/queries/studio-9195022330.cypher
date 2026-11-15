MERGE (d:Document {document_id: $document_id})
ON CREATE SET d.created_at = datetime()
SET d.name = $document_name,
    d.file_name = $document_name,
    d.source_path = $source_path,
    d.document_sha256 = $document_sha256,
    d.page_count = $page_count,
    d.ocr_engine = $ocr_engine,
    d.updated_at = datetime()
