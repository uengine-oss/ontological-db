-- Phase 1. Both extensions carry og_shared's #[pg_schema] module `k0_fixed` (feature fixed-schema).
-- Expected: ext_b cannot be created next to ext_a, because both scripts create the same schema
-- k0_fixed and the same function k0_fixed.k0_in_fixed_schema.
\echo === phase 1: shared #[pg_schema] in both extensions
CREATE EXTENSION ext_a;
CREATE EXTENSION ext_b;
SELECT extname FROM pg_extension WHERE extname LIKE 'ext_%' ORDER BY 1;
DROP EXTENSION IF EXISTS ext_b;
DROP EXTENSION ext_a;
