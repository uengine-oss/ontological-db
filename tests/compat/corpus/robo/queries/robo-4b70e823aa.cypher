MATCH (n) WHERE n.id IN $ids
RETURN collect({labels: labels(n), props: properties(n)}) AS nodes
