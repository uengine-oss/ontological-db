CREATE VECTOR INDEX entity_embedding_vector IF NOT EXISTS
FOR (n:_Entity) ON (n.embedding)
OPTIONS {indexConfig: {
    `vector.dimensions`: 8,
    `vector.similarity_function`: 'cosine'
}}
