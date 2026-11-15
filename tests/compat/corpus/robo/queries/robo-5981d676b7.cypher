MATCH (j:Journey {id: $journey_id})
UNWIND $steps AS st
MERGE (s:JourneyStep {key: st.key})
  ON CREATE SET s.id = st.id, s.createdAt = datetime()
SET s.kind = st.kind,
    s.label = st.label,
    s.sequence = st.sequence,
    s.journeyId = $journey_slug,
    s.source = $source,
    s.updatedAt = datetime()
MERGE (j)-[:HAS_STEP]->(s)
