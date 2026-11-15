MERGE (c:Constitution {id:$id})
SET c.scope='PROJECT', c.raw=$raw, c.updatedAt=$updatedAt, c.principles=$principles
