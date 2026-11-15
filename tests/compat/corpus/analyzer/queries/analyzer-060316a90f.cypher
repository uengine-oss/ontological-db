MATCH (__cy_proc__)
WHERE (__cy_proc__:PROCEDURE OR __cy_proc__:FUNCTION)
  AND __cy_proc__._owner = $analyzer_owner
  AND toLower(__cy_proc__.name) = toLower($proc_name)
MATCH (__cy_t__:TABLE)
WHERE __cy_t__._owner = $analyzer_owner
  AND toLower(__cy_t__.name) = toLower($table_name)
MERGE (__cy_proc__)-[__cy_r__:%s {_owner: $catalog_owner}]->(__cy_t__)
SET __cy_r__.operations = [$operation_type]
RETURN __cy_proc__, __cy_r__, __cy_t__
