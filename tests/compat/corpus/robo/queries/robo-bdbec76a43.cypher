MERGE (h:BpmSession {id: $sid, session_id: $sid}) SET h.bpmn_xml = $xml, h.updated_at = datetime()
