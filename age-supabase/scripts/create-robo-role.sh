#!/usr/bin/env bash
# Create (or re-key) the `robo` login role with a random password, and give it
# what it needs in database DB (default `graph`, the graph database the image
# makes; run again with DB=postgres to use AGE in `postgres` too).
#
#   NAME=agesb-db ADMIN_PW_FILE=... ROBO_PW_FILE=/private/dir/robo.password ./scripts/create-robo-role.sh
#
# The password is written to ROBO_PW_FILE (mode 0600). Only its SCRAM-SHA-256
# verifier is sent to the server. If ROBO_PW_FILE already holds a password it
# is reused, so re-running the script is safe.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
NAME="${NAME:-agesb-db}"
DB="${DB:-graph}"
ADMIN_PW_FILE="${ADMIN_PW_FILE:?path of a file holding the supabase_admin password}"
ROBO_PW_FILE="${ROBO_PW_FILE:?path of the file to keep the robo password in}"

umask 077
mkdir -p "$(dirname "$ROBO_PW_FILE")"
if [ ! -s "$ROBO_PW_FILE" ]; then
  python3 -c 'import secrets; print(secrets.token_urlsafe(24))' > "$ROBO_PW_FILE"
fi
chmod 600 "$ROBO_PW_FILE"

# SCRAM-SHA-256 verifier (RFC 5802 / 7677), the format PostgreSQL stores in pg_authid
verifier="$(python3 -I - "$ROBO_PW_FILE" <<'PY'
import base64, hashlib, hmac, os, sys
pw = open(sys.argv[1]).read().strip().encode()   # token_urlsafe: ASCII, SASLprep is a no-op
salt, it = os.urandom(16), 4096
salted = hashlib.pbkdf2_hmac('sha256', pw, salt, it)
ck = hmac.new(salted, b'Client Key', 'sha256').digest()
sk = hmac.new(salted, b'Server Key', 'sha256').digest()
b = lambda x: base64.b64encode(x).decode()
print(f"SCRAM-SHA-256${it}:{b(salt)}${b(hashlib.sha256(ck).digest())}:{b(sk)}")
PY
)"

docker exec -i -e PGPASSWORD="$(cat "$ADMIN_PW_FILE")" "$NAME" \
  psql -X -q -v ON_ERROR_STOP=1 -U supabase_admin -h 127.0.0.1 -d "$DB" \
  -v robo_verifier="$verifier" < "$here/sql/robo-role.sql"

# prove the password works: loopback is `trust` in the image's pg_hba.conf, so
# connect to the container's own network address, which needs scram-sha-256
docker exec -e PGPASSWORD="$(cat "$ROBO_PW_FILE")" "$NAME" sh -c \
  'psql -X -At -U robo -h "$(hostname -i | cut -d" " -f1)" -d "$0" -c "select '"'"'robo login ok (scram) as '"'"' || current_user || '"'"', search_path='"'"' || current_setting('"'"'search_path'"'"')"' "$DB"
