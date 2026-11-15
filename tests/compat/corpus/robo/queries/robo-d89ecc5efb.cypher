MATCH (f)-[:HAS_BUSINESS_LOGIC]->(bl:BusinessLogic)
WHERE f.id = $fid
RETURN bl.sequence AS seq, bl.title AS title,
       bl.given AS given, bl.when AS wh, bl.then AS th,
       $fid AS function_id, $fid AS function_name,
       '[]' AS write_effects_json
ORDER BY bl.sequence
