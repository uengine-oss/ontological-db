MATCH (n)
WITH labels(n)[0] as label, count(n) as count
RETURN collect({label: label, count: count}) as counts
