MERGE (g:BpmGateway {id: $id, session_id: $sid}) SET g.name = $name, g.gateway_type = $gtype,     g.description = $desc, g.condition_data = $cond,     g.process_id = $pid
