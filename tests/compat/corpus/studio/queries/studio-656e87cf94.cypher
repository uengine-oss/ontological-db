CREATE VECTOR INDEX chunk_embedding_vector IF NOT EXISTS
FOR (c:Chunk) ON (c.embedding)
OPTIONS {
  indexConfig: {
    `vector.dimensions`: 8,
    `vector.similarity_function`: 'cosine'
  }
}
