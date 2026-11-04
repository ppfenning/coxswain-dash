import importlib.util
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("check_clean", HERE / "check_clean.py")
check_clean = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_clean)

DENY = {"COXSWAIN_CLEAN_DENY": "zebrafish"}
# Built at run time so this file carries no literal the rules would flag.
HOME_LINE = "path = /" + "home" + "/someone/.cargo/registry\n"


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def run(capsys, argv, environ):
    code = check_clean.main(["check_clean.py", *argv], environ)
    return code, capsys.readouterr().out


def test_a_file_naming_the_deny_regex_fails_with_file_and_line(tmp_path, capsys):
    write(tmp_path, "notes.txt", "fine\nthe zebrafish line\n")
    code, out = run(capsys, [str(tmp_path)], DENY)
    assert code == 1
    assert "notes.txt:2: names: 'zebrafish'" in out


def test_require_names_without_the_variable_returns_2(tmp_path, capsys):
    write(tmp_path, "notes.txt", "the zebrafish line\n")
    for environ in ({}, {"COXSWAIN_CLEAN_DENY": ""}):
        code, out = run(capsys, [str(tmp_path), "--require-names"], environ)
        assert code == 2
        assert out.splitlines() == ["names rule cannot run: COXSWAIN_CLEAN_DENY is not set"]


def test_without_the_flag_and_the_variable_the_names_rule_is_skipped(tmp_path, capsys):
    write(tmp_path, "notes.txt", "the zebrafish line\n")
    code, out = run(capsys, [str(tmp_path)], {})
    assert code == 0
    assert "names rule skipped: COXSWAIN_CLEAN_DENY is not set" in out


def test_the_flag_may_come_before_or_after_the_root(tmp_path):
    expected = (tmp_path, True)
    assert check_clean.split_args(["p", str(tmp_path), "--require-names"]) == expected
    assert check_clean.split_args(["p", "--require-names", str(tmp_path)]) == expected
    assert check_clean.split_args(["p"]) == (pathlib.Path("."), False)


def test_require_names_with_the_variable_set_scans_normally(tmp_path, capsys):
    write(tmp_path, "notes.txt", "clean text\n")
    code, _ = run(capsys, ["--require-names", str(tmp_path)], DENY)
    assert code == 0


def test_a_top_level_target_directory_is_ignored_but_a_nested_one_is_not(tmp_path, capsys):
    write(tmp_path, "target/debug/deps/x.d", HOME_LINE)
    code, out = run(capsys, [str(tmp_path)], {})
    assert code == 0
    write(tmp_path, "src/target/x.d", HOME_LINE)
    code, out = run(capsys, [str(tmp_path)], {})
    assert code == 1
    assert "src/target/x.d:1: home-path" in out
    assert "debug" not in out


def test_the_repository_tree_is_clean(capsys):
    code, out = run(capsys, [str(HERE.parent)], {})
    assert code == 0, out
