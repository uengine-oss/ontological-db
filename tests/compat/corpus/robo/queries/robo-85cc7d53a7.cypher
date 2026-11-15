MATCH (us:UserStory {id: $usid})
OPTIONAL MATCH (us)-[old:IMPLEMENTS]->(:BoundedContext)
DELETE old
