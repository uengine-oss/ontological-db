MATCH (m:StoryboardPageMapping {status: 'active'})
SET m.status = 'archived'
RETURN count(m) AS n
