MATCH (f {_owner: 'analyzer'})-[:HAS_RULE]->(r:RULE {_owner: 'analyzer'}) RETURN r.condition_description AS condition, coalesce(r.effect_descriptions, []) AS effects
