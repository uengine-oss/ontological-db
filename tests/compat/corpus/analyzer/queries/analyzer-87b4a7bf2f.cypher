CREATE VECTOR INDEX analyzer_search_vec IF NOT EXISTS FOR (n:ANALYZER_OWNED) ON (n.search_embedding) OPTIONS {indexConfig: {`vector.dimensions`: 8, `vector.similarity_function`: 'cosine'}}
