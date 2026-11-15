MATCH (p:Process) WHERE p.proc_id IN $pids AND p.session_id IS NULL SET p:BpmnProcess, p.session_id = $sid REMOVE p:Process
