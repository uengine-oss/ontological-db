UNWIND $rows AS r
MATCH (evt:Event {id: r.evt_id})
SET evt.sequence = r.sequence,
    evt.updatedAt = datetime()
RETURN {evt_id: evt.id, sequence: evt.sequence} AS result
