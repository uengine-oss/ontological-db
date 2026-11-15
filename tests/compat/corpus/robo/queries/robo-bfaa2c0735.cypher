MATCH (j:Journey {id: $journey_id})-[:HAS_STEP]->(st:JourneyStep)
DETACH DELETE st
