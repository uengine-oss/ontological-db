MATCH (__cy_t1__:TABLE)-[__cy_r__:FK]->(__cy_t2__:TABLE)
WHERE __cy_t1__._owner = $owner
  AND __cy_t2__._owner = $owner
  AND (__cy_t1__.name = $table_name OR __cy_t2__.name = $table_name
    OR __cy_t1__._id ENDS WITH $table_name OR __cy_t2__._id ENDS WITH $table_name)
RETURN __cy_t1__.name AS from_table, 
       COALESCE(__cy_t1__.schema_name, __cy_t1__.schema) AS from_schema,
       __cy_t1__.description AS from_desc,
       __cy_t2__.name AS to_table, 
       COALESCE(__cy_t2__.schema_name, __cy_t2__.schema) AS to_schema,
       __cy_t2__.description AS to_desc,
       __cy_r__.from_column AS source_column,
       __cy_r__.to_column AS target_column,
       __cy_r__._owner AS source,
       type(__cy_r__) AS rel_type
