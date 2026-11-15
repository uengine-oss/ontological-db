CREATE CONSTRAINT customer_customer_id_key IF NOT EXISTS FOR (n:`Customer`) REQUIRE n.`customer_id` IS UNIQUE
