#!/bin/bash
# Offline-consistent SQLite backups (audit.db) via the sqlite3 backup API.
# Usage: backup-audit.sh <data-dir> <backup-dir> [keep-days=7]
# Safe on live databases (no file copy races); verifies integrity after.
set -eu
DATA_DIR="${1:?data dir}"; BACKUP_DIR="${2:?backup dir}"; KEEP="${3:-7}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$BACKUP_DIR"
for db in "$DATA_DIR"/audit.db "$DATA_DIR"/../data-rust/audit.db; do
  [ -f "$db" ] || continue
  name="$(basename "$(dirname "$db")")-audit"
  dest="$BACKUP_DIR/${name}-${STAMP}.db"
  python3 - "$db" "$dest" <<'EOF'
import sqlite3, sys
src = sqlite3.connect(sys.argv[1])
dst = sqlite3.connect(sys.argv[2])
with dst:
    src.backup(dst)
ok = dst.execute("PRAGMA integrity_check").fetchone()[0]
assert ok == "ok", ok
n = dst.execute("select count(*) from audit").fetchone()[0]
print(f"backup {sys.argv[2]}: {n} rows, integrity ok")
dst.close(); src.close()
EOF
  gzip -f "$dest"
done
# retention
find "$BACKUP_DIR" -name '*-audit-*.db.gz' -mtime +"$KEEP" -delete
echo "retention: keep last $KEEP days in $BACKUP_DIR"
