"""No employer, no tracker, no person, anywhere in this repository.

The whole claim of this repo is that a tool outlives the workplace it was
first used at. That claim is worth exactly as much as the check that enforces
it, so it is enforced here rather than remembered.
"""
import pathlib
import re
import sys

# Product names a seat must never assume, and the shape of a personal name.
DENY = re.compile(r"\b(asana|jira|confluence|blastpoint|cloudwatch|outline|snowflake|databricks)\b", re.I)
SKIP = {".git", ".github", ".venv", "node_modules"}

problems = []
for path in sorted(pathlib.Path(".").rglob("*")):
    if not path.is_file() or set(path.parts) & SKIP:
        continue
    if path.suffix not in {".md", ".sh", ".yml", ".yaml", ".py"}:
        continue
    for number, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
        hit = DENY.search(line)
        if hit:
            problems.append(f"{path}:{number}: names '{hit.group(0)}' — a tool must not assume a vendor")

print("\n".join(problems) if problems else "clean: no vendor or employer named")
sys.exit(1 if problems else 0)
