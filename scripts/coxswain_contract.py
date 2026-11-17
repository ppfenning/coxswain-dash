"""Towpath's tests read coxswain's dash fixtures at one pinned coxswain commit, never hand copies.

`coxswain-contract.toml` names the commit and the contract version it carries. `tests/fixtures/coxswain/` holds the
fixtures vendored from that commit, byte for byte, with coxswain's `dash_contract.json` manifest that lists them.

    python3 scripts/coxswain_contract.py verify      # CI: the vendored set is exactly the set at the pin
    python3 scripts/coxswain_contract.py sync <ref>  # the only way to take new fixtures: re-vendor and move the pin

`sync` takes a branch, tag or commit, resolves it to a commit and records that, so the pin never moves on its own.
The functions above `# edge` are pure and tested in `scripts/test_coxswain_contract.py`; the edge below fetches over
HTTPS and runs `git ls-remote`, each with a timeout."""

from __future__ import annotations

import json
import re
import subprocess
import sys
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping

ROOT = Path(__file__).resolve().parent.parent
PIN_FILE = ROOT / "coxswain-contract.toml"
VENDOR_DIR = ROOT / "tests" / "fixtures" / "coxswain"
UPSTREAM_DIR = "src/tools/tests/fixtures"
MANIFEST = "dash_contract.json"
FETCH_TIMEOUT_S = 30
LS_REMOTE_TIMEOUT_S = 60

PIN_HEADER = (
    "# The coxswain commit whose dash fixtures towpath's tests read, and the contract version they carry.\n"
    "# tests/fixtures/coxswain/ is vendored from this commit and CI fails if it differs. Never edit it or this file\n"
    "# by hand: `python3 scripts/coxswain_contract.py sync <ref>` re-vendors both together.\n"
)
_SHA = re.compile(r"[0-9a-f]{40}")
_LINE = re.compile(r'^\s*(\w+)\s*=\s*(?:"([^"]*)"|(\d+))\s*$')


@dataclass(frozen=True)
class Pin:
    repo: str
    ref: str
    contract: int


def parse_pin(text: str) -> Pin:
    """The pin from `coxswain-contract.toml`'s text: `repo` and `ref` strings and an integer `contract`. `ref` must
    be a full commit, so the fixtures a pin names cannot change under it. Raises ValueError saying what is wrong."""
    fields = {
        match.group(1): match.group(2) if match.group(2) is not None else int(match.group(3))
        for line in text.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
        for match in [_LINE.match(line)]
        if match
    }
    missing = [key for key in ("repo", "ref", "contract") if key not in fields]
    if missing:
        raise ValueError(f"the pin file lacks {', '.join(missing)}")
    if not isinstance(fields["repo"], str) or not isinstance(fields["ref"], str):
        raise ValueError("the pin's repo and ref are quoted strings")
    if not _SHA.fullmatch(fields["ref"]):
        raise ValueError(f"the pin's ref {fields['ref']!r} is not a full commit sha")
    if not isinstance(fields["contract"], int):
        raise ValueError("the pin's contract is an integer")
    return Pin(fields["repo"], fields["ref"], fields["contract"])


def render_pin(pin: Pin) -> str:
    """The pin file's text for `pin`; `parse_pin(render_pin(pin)) == pin`."""
    return f'{PIN_HEADER}repo = "{pin.repo}"\nref = "{pin.ref}"\ncontract = {pin.contract}\n'


def parse_manifest(text: str) -> tuple[int, tuple[str, ...]]:
    """Coxswain's `dash_contract.json`: its contract version and the fixture file names it lists."""
    body = json.loads(text)
    names = body.get("fixtures") if isinstance(body, dict) else None
    contract = body.get("contract") if isinstance(body, dict) else None
    if not isinstance(contract, int) or not isinstance(names, list) or not all(isinstance(n, str) for n in names):
        raise ValueError("the manifest needs an integer `contract` and a `fixtures` list of file names")
    bad = [name for name in names if "/" in name or not name.endswith(".json")]
    if bad:
        raise ValueError(f"the manifest lists names that are not plain .json files: {bad}")
    return contract, tuple(names)


def upstream_url(repo: str, ref: str, name: str) -> str:
    """Where fixture `name` lives at `ref` in `repo`, as raw file content."""
    return f"https://raw.githubusercontent.com/{repo}/{ref}/{UPSTREAM_DIR}/{name}"


def drift(vendored: Mapping[str, bytes], upstream: Mapping[str, bytes]) -> tuple[str, ...]:
    """Each way the vendored files differ from the files at the pin, by name; empty when they are the same set."""
    changed = [f"{name} differs from the pinned commit's copy" for name in sorted(vendored.keys() & upstream.keys())
               if vendored[name] != upstream[name]]
    missing = [f"{name} is at the pinned commit but not vendored" for name in sorted(upstream.keys() - vendored.keys())]
    extra = [f"{name} is vendored but not at the pinned commit" for name in sorted(vendored.keys() - upstream.keys())]
    return tuple(changed + missing + extra)


def contract_findings(pin: Pin, manifest_contract: int) -> tuple[str, ...]:
    """The pin must record the contract the pinned commit publishes; empty when it does."""
    if pin.contract == manifest_contract:
        return ()
    return (f"the pin records contract {pin.contract} but coxswain at {pin.ref[:12]} publishes contract "
            f"{manifest_contract}; run sync, then update towpath's CONTRACT_VERSION and its parser",)


def resolved_sha(ls_remote_output: str, ref: str) -> str | None:
    """The commit `git ls-remote <url> <ref>` names for `ref`: the exact `refs/heads|tags/<ref>` row, the peeled tag
    when there is one; `ref` itself when it already is a full sha; None when nothing matched."""
    if _SHA.fullmatch(ref):
        return ref
    rows = dict(
        (name, sha) for sha, _, name in (line.partition("\t") for line in ls_remote_output.splitlines()) if name
    )
    for name in (f"refs/tags/{ref}^{{}}", f"refs/tags/{ref}", f"refs/heads/{ref}", ref):
        if name in rows:
            return rows[name]
    return None


# edge
def _fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=FETCH_TIMEOUT_S) as response:
        return response.read()


def fetch_at(repo: str, ref: str) -> dict[str, bytes]:
    """The manifest and every fixture it lists, at `ref`, by file name."""
    manifest = _fetch(upstream_url(repo, ref, MANIFEST))
    _, names = parse_manifest(manifest.decode("utf-8"))
    return {MANIFEST: manifest, **{name: _fetch(upstream_url(repo, ref, name)) for name in names}}


def read_vendored(folder: Path) -> dict[str, bytes]:
    return {path.name: path.read_bytes() for path in sorted(folder.glob("*.json"))} if folder.is_dir() else {}


def resolve(repo: str, ref: str) -> str:
    if _SHA.fullmatch(ref):
        return ref
    done = subprocess.run(
        ["git", "ls-remote", f"https://github.com/{repo}.git", ref],
        capture_output=True, text=True, timeout=LS_REMOTE_TIMEOUT_S, check=False,
    )
    sha = resolved_sha(done.stdout, ref) if done.returncode == 0 else None
    if sha is None:
        raise SystemExit(f"cannot resolve {ref!r} in {repo}: {done.stderr.strip() or 'no such ref'}")
    return sha


def verify() -> int:
    pin = parse_pin(PIN_FILE.read_text(encoding="utf-8"))
    upstream = fetch_at(pin.repo, pin.ref)
    manifest_contract, _ = parse_manifest(upstream[MANIFEST].decode("utf-8"))
    findings = drift(read_vendored(VENDOR_DIR), upstream) + contract_findings(pin, manifest_contract)
    for finding in findings:
        print(f"coxswain contract: {finding}")
    if findings:
        print("Take coxswain's fixtures only with `python3 scripts/coxswain_contract.py sync <ref>`.")
        return 1
    print(f"coxswain contract {pin.contract}: {len(upstream)} files match {pin.repo}@{pin.ref[:12]}")
    return 0


def sync(ref: str) -> int:
    current = parse_pin(PIN_FILE.read_text(encoding="utf-8"))
    sha = resolve(current.repo, ref)
    upstream = fetch_at(current.repo, sha)
    manifest_contract, _ = parse_manifest(upstream[MANIFEST].decode("utf-8"))
    VENDOR_DIR.mkdir(parents=True, exist_ok=True)
    for stale in read_vendored(VENDOR_DIR).keys() - upstream.keys():
        (VENDOR_DIR / stale).unlink()
    for name, body in upstream.items():
        (VENDOR_DIR / name).write_bytes(body)
    PIN_FILE.write_text(render_pin(Pin(current.repo, sha, manifest_contract)), encoding="utf-8")
    print(f"pinned {current.repo}@{sha[:12]}, contract {manifest_contract}, {len(upstream)} files")
    return 0


def main(argv: list[str]) -> int:
    if argv[1:2] == ["verify"] and len(argv) == 2:
        return verify()
    if argv[1:2] == ["sync"] and len(argv) == 3:
        return sync(argv[2])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
