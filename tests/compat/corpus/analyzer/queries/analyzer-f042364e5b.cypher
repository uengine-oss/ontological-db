MATCH (__cy_n__)
WHERE __cy_n__._owner = 'analyzer'
RETURN elementId(__cy_n__) AS nodeId, labels(__cy_n__) AS labels, __cy_n__{.*, embedding: null} AS props
