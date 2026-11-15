MATCH (e:Event {id: $id}) SET e.pivotal = $pivotal RETURN coalesce(e.pivotal, false) AS pivotal, coalesce(e.hotspot, false) AS hotspot
