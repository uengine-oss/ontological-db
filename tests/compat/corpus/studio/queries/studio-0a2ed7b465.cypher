MATCH (p:_Entity:MfgProduct {product_id: $pid})
      -[:REFERENCES_MFG_PRODUCT]-(r:_Entity:MfgProductRouting)
      -[:REFERENCES_MFG_PROCESS_STEP]-(s:_Entity:MfgProcessStep)
RETURN s.step_id AS step_id, s.step_name AS step_name
