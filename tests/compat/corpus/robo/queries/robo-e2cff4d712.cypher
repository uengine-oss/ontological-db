MATCH (p:BpmProcess {id: $id, session_id: $sid}) SET p.bpmn_xml = $xml
