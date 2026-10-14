#!/usr/bin/env bash
# Run every tutorial as a regression test.
#
# A tutorial's examples are executed, not pasted: each one is a statement and,
# where it matters, an assertion on what comes back. Running them all before a
# release is the check that what the documentation shows is what the database
# does. Nothing is written; exit status is non-zero if any example fails.
#
#   ./tests/tutorials.sh            # the tutorials that need only PostgreSQL
#   ./tests/tutorials.sh --strict   # also fail when output differs from the published page
#   FABRIC=1 ./tests/tutorials.sh   # also the virtual-fabric tutorial, which needs its two
#                                   # source databases and a Studio (see that tutorial's "준비")
#
# Env: TUT_EXEC (default "docker exec -i og-fabric"), TUT_PGPORT (default 28816).
set -u
cd "$(dirname "$0")/.."
status=0
python3 examples/tutorials/build_tutorial.py --check --all "$@" || status=1
if [ "${FABRIC:-0}" = "1" ]; then
    echo
    python3 examples/tutorials/virtual_fabric/build.py --check || status=1
fi
exit $status
