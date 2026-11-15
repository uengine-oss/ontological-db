MATCH (__cy_f__)-[__cy_r__:READS|WRITES]->(__cy_t__:TABLE)
WHERE __cy_f__._owner = 'analyzer'
  AND __cy_t__._owner = 'analyzer'
RETURN elementId(__cy_f__) AS fid,
       coalesce(__cy_f__.logical_name, __cy_f__.name, __cy_f__._id) AS fname,
       type(__cy_r__) AS rel,
       elementId(__cy_t__) AS tid,
       coalesce(__cy_t__.logical_name, __cy_t__.name, __cy_t__._id) AS tname
