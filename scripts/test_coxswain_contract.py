"""The pure core of scripts/coxswain_contract.py, on example inputs. Runs under `python3 -m unittest` and pytest."""

import importlib.util
import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("coxswain_contract", HERE / "coxswain_contract.py")
contract = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = contract  # the dataclass reads its module back while it is built
SPEC.loader.exec_module(contract)

SHA_A = "a" * 40
SHA_B = "b" * 40
PIN = contract.Pin("owner/coxswain", SHA_A, 1)
FEED_AT_A = b'{"schema": 1, "spend": {"five_hour_fraction": 0.1}}\n'
FEED_AT_B = b'{"schema": 1, "spend": {"five_hour_share": 0.1}}\n'
MANIFEST = b'{"contract": 1, "fixtures": ["dash_feed_v1.json"]}\n'


class PinFile(unittest.TestCase):
    def test_a_rendered_pin_parses_back_to_itself(self):
        self.assertEqual(contract.parse_pin(contract.render_pin(PIN)), PIN)

    def test_comments_and_spacing_are_ignored(self):
        text = f'# note\n\nrepo="owner/coxswain"\n  ref = "{SHA_A}"\ncontract=1\n'
        self.assertEqual(contract.parse_pin(text), PIN)

    def test_a_branch_name_is_refused_so_the_pin_cannot_move_on_its_own(self):
        with self.assertRaisesRegex(ValueError, "not a full commit sha"):
            contract.parse_pin('repo = "owner/coxswain"\nref = "main"\ncontract = 1\n')

    def test_a_missing_field_is_named(self):
        with self.assertRaisesRegex(ValueError, "lacks contract"):
            contract.parse_pin(f'repo = "owner/coxswain"\nref = "{SHA_A}"\n')


class Manifest(unittest.TestCase):
    def test_reads_the_contract_and_the_names(self):
        self.assertEqual(contract.parse_manifest(MANIFEST.decode()), (1, ("dash_feed_v1.json",)))

    def test_a_name_with_a_path_is_refused(self):
        with self.assertRaisesRegex(ValueError, "not plain .json files"):
            contract.parse_manifest('{"contract": 1, "fixtures": ["../secrets.json"]}')

    def test_a_manifest_without_a_contract_is_refused(self):
        with self.assertRaisesRegex(ValueError, "integer `contract`"):
            contract.parse_manifest('{"fixtures": []}')

    def test_each_fixture_resolves_to_its_raw_path_at_the_ref(self):
        self.assertEqual(
            contract.upstream_url("owner/coxswain", SHA_A, "dash_feed_v1.json"),
            f"https://raw.githubusercontent.com/owner/coxswain/{SHA_A}/src/tools/tests/fixtures/dash_feed_v1.json",
        )


class Drift(unittest.TestCase):
    def test_the_vendored_set_equal_to_the_pin_has_no_drift(self):
        files = {"dash_contract.json": MANIFEST, "dash_feed_v1.json": FEED_AT_A}
        self.assertEqual(contract.drift(files, dict(files)), ())

    def test_a_fixture_changed_at_a_different_pin_is_drift_until_towpath_resyncs(self):
        """The done-when, simulated: coxswain renames a key at commit B. Towpath vendored at A still differs from
        B, so verifying against B fails; only a sync to B (and a towpath change that reads the new key) clears it."""
        vendored_at_a = {"dash_contract.json": MANIFEST, "dash_feed_v1.json": FEED_AT_A}
        upstream_at_b = {"dash_contract.json": MANIFEST, "dash_feed_v1.json": FEED_AT_B}
        self.assertEqual(
            contract.drift(vendored_at_a, upstream_at_b), ("dash_feed_v1.json differs from the pinned commit's copy",)
        )

    def test_a_hand_edit_to_a_vendored_fixture_is_drift(self):
        upstream = {"dash_feed_v1.json": FEED_AT_A}
        edited = {"dash_feed_v1.json": FEED_AT_A.replace(b"0.1", b"0.2")}
        self.assertEqual(contract.drift(edited, upstream), ("dash_feed_v1.json differs from the pinned commit's copy",))

    def test_added_and_removed_fixtures_are_drift(self):
        self.assertEqual(
            contract.drift({"dash_old_v1.json": b"{}"}, {"dash_new_v1.json": b"{}"}),
            ("dash_new_v1.json is at the pinned commit but not vendored",
             "dash_old_v1.json is vendored but not at the pinned commit"),
        )


class ContractVersion(unittest.TestCase):
    def test_the_same_contract_has_no_finding(self):
        self.assertEqual(contract.contract_findings(PIN, 1), ())

    def test_a_newer_contract_at_the_pin_is_a_finding(self):
        (finding,) = contract.contract_findings(PIN, 2)
        self.assertIn("records contract 1 but coxswain at aaaaaaaaaaaa publishes contract 2", finding)


class ResolveRef(unittest.TestCase):
    LS_REMOTE = f"{SHA_A}\trefs/heads/main\n{SHA_B}\trefs/tags/v1\n{'c' * 40}\trefs/tags/v1^{{}}\n"

    def test_a_branch_resolves_to_its_head(self):
        self.assertEqual(contract.resolved_sha(self.LS_REMOTE, "main"), SHA_A)

    def test_an_annotated_tag_resolves_to_its_commit(self):
        self.assertEqual(contract.resolved_sha(self.LS_REMOTE, "v1"), "c" * 40)

    def test_a_full_sha_is_itself(self):
        self.assertEqual(contract.resolved_sha("", SHA_B), SHA_B)

    def test_an_unknown_ref_is_none(self):
        self.assertIsNone(contract.resolved_sha(self.LS_REMOTE, "nope"))


if __name__ == "__main__":
    unittest.main()
