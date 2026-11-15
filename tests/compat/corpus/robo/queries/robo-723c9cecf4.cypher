MATCH (p:BpmProcess {session_id: $sid})
WITH count(p) AS processes
MATCH (t:BpmTask {session_id: $sid})
WITH processes, count(t) AS tasks
MATCH (a:BpmActor {session_id: $sid})
RETURN processes, tasks, count(a) AS actors
