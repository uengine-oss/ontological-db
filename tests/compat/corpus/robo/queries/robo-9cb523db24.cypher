MATCH (bc:BoundedContext {id: $bc_id})<-[:IMPLEMENTS]-(linked_us:UserStory)
WITH bc, collect(DISTINCT linked_us.id) as all_us_ids
SET bc.userStoryIds = all_us_ids,
    bc.updatedAt = datetime()
RETURN bc.id, all_us_ids
