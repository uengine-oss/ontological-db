MATCH (e:Event)
RETURN coalesce(e.displayName, e.name) AS name,
       coalesce(e.sequence, 0) AS seq,
       coalesce(e.pivotal, false) AS pivotal,
       coalesce(e.hotspot, false) AS hotspot
ORDER BY seq, name
