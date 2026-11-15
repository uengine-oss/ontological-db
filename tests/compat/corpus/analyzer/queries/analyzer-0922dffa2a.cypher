MATCH (__cy_src__:TABLE)
WHERE __cy_src__._owner = $analyzer_owner
  AND toLower(__cy_src__.name) = toLower($src_name)
MATCH (__cy_tgt__:TABLE)
WHERE __cy_tgt__._owner = $analyzer_owner
  AND toLower(__cy_tgt__.name) = toLower($tgt_name)
MERGE (__cy_src__)-[__cy_r__:DATA_FLOWS_TO {_owner: $catalog_owner, via: $etl_name}]->(__cy_tgt__)
SET __cy_r__.description = $operation_type
RETURN __cy_src__, __cy_r__, __cy_tgt__
