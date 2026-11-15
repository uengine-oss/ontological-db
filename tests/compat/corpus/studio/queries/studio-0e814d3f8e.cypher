CREATE FULLTEXT INDEX chunk_source_text_fulltext IF NOT EXISTS
FOR (c:Chunk) ON EACH [c.source_text]
