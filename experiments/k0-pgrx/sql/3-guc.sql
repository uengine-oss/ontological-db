-- Phase 3. og_shared defines GUC k0.shared_probe the first time k0_guc_probe() runs, the way
-- engine/src/fabric/mod.rs:370 defines ontological.fabric_inline_passwords. Each shared object
-- has its own "already defined" flag, so the second one tries to define the same name again.
\echo === phase 3: a GUC defined by shared code, used from both extensions in one backend
SELECT public.k0_guc_probe() AS from_ext_a;
SELECT ext_b_s.k0_guc_probe() AS from_ext_b;
SELECT ext_b_s.k0_guc_probe() AS from_ext_b_again;
SHOW k0.shared_probe;
