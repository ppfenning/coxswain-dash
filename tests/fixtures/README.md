# Fixtures

- `coxswain/` holds coxswain's dash fixtures, vendored byte for byte from the commit that
  `coxswain-contract.toml` pins, along with `dash_contract.json`, the manifest that lists them and
  the contract version. Never edit these files by hand. CI runs
  `python3 scripts/coxswain_contract.py verify` and fails when they differ from the pinned commit.
  To take new fixtures, run `python3 scripts/coxswain_contract.py sync <ref>`. It re-vendors the
  files and moves the pin together, and the same pull request adapts towpath to whatever changed.
- `towpath/` holds towpath's own scenarios: inputs that its rendering tests pin values against. These are
  not copies of coxswain's fixtures. `contract::tests` fails if one of them uses a key that no
  pinned fixture fixes.
