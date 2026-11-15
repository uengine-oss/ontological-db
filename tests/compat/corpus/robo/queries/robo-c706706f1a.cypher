UNWIND $edges AS e
MATCH (a:JourneyStep {id: e.source_step_id})
MATCH (b:JourneyStep {id: e.target_step_id})
MERGE (a)-[r:NEXT {id: e.id}]->(b)
  ON CREATE SET r.createdAt = datetime()
SET r.condition = coalesce(e.condition, ''),
    r.documentExcerpt = coalesce(e.document_excerpt, ''),
    r.source = coalesce(e.source, 'llm'),
    r.updatedAt = datetime()
