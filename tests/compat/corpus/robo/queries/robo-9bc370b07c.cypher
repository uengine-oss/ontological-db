MERGE (c:Constitution {id:$id})
SET c.scope='PROJECT', c.updatedAt=$updatedAt,
    c.strategicMemory = $mem
