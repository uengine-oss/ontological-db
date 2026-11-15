MATCH (e:BindingHistoryEvent)
RETURN e
ORDER BY e.at DESC
LIMIT $lim
