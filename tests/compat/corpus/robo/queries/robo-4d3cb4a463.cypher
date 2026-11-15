MATCH (us:UserStory {session_id: $sid})
WITH count(us) AS user_stories
MATCH (bc:BoundedContext {session_id: $sid})
WITH user_stories, count(bc) AS bounded_contexts
MATCH (agg:Aggregate {session_id: $sid})
WITH user_stories, bounded_contexts, count(agg) AS aggregates
MATCH (cmd:Command {session_id: $sid})
WITH user_stories, bounded_contexts, aggregates, count(cmd) AS commands
MATCH (evt:Event {session_id: $sid})
WITH user_stories, bounded_contexts, aggregates, commands, count(evt) AS events
MATCH (pol:Policy {session_id: $sid})
WITH user_stories, bounded_contexts, aggregates, commands, events, count(pol) AS policies
MATCH (rm:ReadModel {session_id: $sid})
RETURN user_stories, bounded_contexts, aggregates, commands, events, policies, count(rm) AS readmodels
