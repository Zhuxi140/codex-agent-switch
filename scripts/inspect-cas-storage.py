"""只读定位 CAS 数据库；不创建数据库、不读取密钥、不修改配置。"""

import contextlib
import json
import os
import pathlib
import sqlite3
import sys


directories = [pathlib.Path.home() / ".codex-agent-switch"]
for variable in ("LOCALAPPDATA", "APPDATA"):
    directory = os.environ.get(variable)
    if directory:
        directories.append(pathlib.Path(directory))

results = []
for directory in directories:
    for path in sorted(directory.glob("com.codexagentswitch.*/cas.db")):
        metadata = path.stat()
        result = {
            "path": str(path),
            "bytes": metadata.st_size,
            "created_at": metadata.st_ctime,
            "modified_at": metadata.st_mtime,
            "related_files": [
                {"name": item.name, "bytes": item.stat().st_size}
                for item in sorted(path.parent.iterdir())
                if item.is_file()
                and item.name.endswith((".db", ".db-wal", ".db-shm", ".bak", ".backup"))
            ],
        }
        try:
            with contextlib.closing(
                sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=2)
            ) as connection:
                connection.execute("PRAGMA query_only = ON")
                tables = {
                    row[0]
                    for row in connection.execute(
                        "SELECT name FROM sqlite_master WHERE type = 'table'"
                    )
                }
                result["tables"] = sorted(tables)
                result["integrity"] = connection.execute("PRAGMA quick_check").fetchone()[0]
                result["counts"] = {
                    table: connection.execute(
                        "SELECT COUNT(*) FROM " + table
                    ).fetchone()[0]
                    for table in ("providers", "models", "agents")
                    if table in tables
                }
                if "schema_migrations" in tables:
                    result["schema_version"] = connection.execute(
                        "SELECT MAX(version) FROM schema_migrations"
                    ).fetchone()[0]
                    result["recent_migrations"] = connection.execute(
                        "SELECT version, name, applied_at FROM schema_migrations "
                        "ORDER BY version DESC LIMIT 5"
                    ).fetchall()
        except sqlite3.Error as error:
            result["error"] = str(error)
        results.append(result)

if len(sys.argv) > 1:
    # 隐藏诊断进程可另存报告；只允许新文件，不覆盖既有报告。
    with open(sys.argv[1], "x", encoding="utf-8") as report:
        json.dump(results, report, ensure_ascii=False)
else:
    for result in results:
        print(json.dumps(result, ensure_ascii=False))
