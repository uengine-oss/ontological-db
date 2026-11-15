MATCH (n:Process) WHERE n.proc_id IS NOT NULL AND n.session_id IS NULL SET n:BpmnProcess, n.session_id = $sid REMOVE n:Process RETURN count(n) AS c
