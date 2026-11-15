MATCH (us:UserStory {id: $us_id})<-[r:HAS_USER_STORY]-(:Feature)
WHERE r.source = 'manual'
RETURN count(r) AS c
