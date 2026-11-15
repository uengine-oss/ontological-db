MATCH (agg:Aggregate)-[:HAS_INVARIANT]->(inv:Invariant {id: $id})
OPTIONAL MATCH (inv)-[:VERIFIED_BY]->(c:Command)
OPTIONAL MATCH (c)-[:HAS_GWT]->(cg:GWT)
WITH agg, inv,
     collect(DISTINCT CASE WHEN c IS NULL THEN NULL
        ELSE {commandId: c.id, commandName: c.name, hasGwt: cg IS NOT NULL} END) AS refs
OPTIONAL MATCH (inv)-[:HAS_GWT]->(og:GWT)
RETURN agg {.id, .name} AS agg, inv {.id, .key, .name, .declaration, .description, .source, .seq} AS inv,
       refs, (og IS NOT NULL) AS hasOwnGwt
