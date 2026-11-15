MATCH (__cy_a__)-[__cy_r__]->(__cy_b__)
WHERE __cy_a__._owner = 'analyzer' AND __cy_b__._owner = 'analyzer'
RETURN elementId(__cy_r__) AS relId, 
       elementId(__cy_a__) AS startId, 
       elementId(__cy_b__) AS endId, 
       type(__cy_r__) AS relType, 
       __cy_r__{.*, embedding: null} AS props
