MATCH (srcEvt:Event)-[:TRIGGERS]->(pol:Policy)-[:INVOKES]->(tgtCmd:Command)
RETURN srcEvt.id AS sourceEventId, tgtCmd.id AS targetCommandId
