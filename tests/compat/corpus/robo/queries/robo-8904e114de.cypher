UNWIND $rows AS r
MATCH (us:UserStory {id: r.us_id})
SET us.sequence = r.sequence,
    us.updatedAt = datetime()
RETURN {us_id: us.id, sequence: us.sequence} AS result
