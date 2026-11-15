MATCH (s:JourneyStep {id: $sid}), (ui:UI)-[:ATTACHED_TO]->(d {id: $did}) MERGE (s)-[:SHOWS]->(ui)
