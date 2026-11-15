MATCH (n:`Customer`) WHERE toString(n.`customer_id`) = $key SET n.`grade` = $value
