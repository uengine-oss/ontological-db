MATCH (n {graph_owner: 'analyzer'}) WHERE n.summary IS NOT NULL AND n.summary <> '' AND NOT n:FILE AND NOT n:MODULE AND NOT n:PACKAGE RETURN count(n) AS c
