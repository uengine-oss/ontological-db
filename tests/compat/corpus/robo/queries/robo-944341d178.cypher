MATCH (chg:RequirementChange {id: $id})-[:EFFECT]->(n)
OPTIONAL MATCH (n)-[:IMPLEMENTS|HAS_AGGREGATE|HAS_COMMAND*1..3]->(design)
OPTIONAL MATCH (test:Test)-[:TESTS_FOR]->(design)
RETURN n.id AS nodeId, labels(n)[0] AS nodeLabel,
       design.id AS designId, labels(design)[0] AS designLabel,
       test.id AS testId, test.type AS testType, test.description AS testDesc
