#!/usr/bin/env python3

# Copyright 2024 tison <wander4096@gmail.com>
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

from pathlib import Path
import difflib
import subprocess
import os
import shutil
import tempfile
import datetime

def diff_files(file1, file2):
    with file1.open("r", encoding="utf8") as f1, file2.open("r", encoding="utf8") as f2:
        diff = difflib.unified_diff(f1.readlines(), f2.readlines(), str(file1), str(file2))
        diff = list(diff)
        if diff:
            for line in diff:
                print(line, end="")
            exit(1)


basedir = Path(__file__).parent.absolute()
rootdir = basedir.parent

subprocess.run(["cargo", "build", "--bin", "hawkeye"], cwd=rootdir, check=True)
hawkeye = rootdir / "target" / "debug" / "hawkeye"

def drive(name, files, create_temp_copy=False):
    temp_paths = []
    expected_files = []
    case_dir = basedir / name
    try:
        if create_temp_copy:
            current_year = str(datetime.datetime.now().year)
            for filepath in files:
                base, ext = os.path.splitext(filepath)
                temp_path = f"{base}_temp{ext}"
                shutil.copy2(case_dir / filepath, case_dir / temp_path)
                temp_paths.append(temp_path)
                expected_temp_path = f"{base}_temp{ext}.expected"
                shutil.copy2(case_dir / f"{filepath}.expected", case_dir / expected_temp_path)
                expected_files.append(expected_temp_path)
                with (case_dir / expected_temp_path).open("r", encoding="utf8") as f:
                    content = f.read()
                content = content.replace("<CURRENT_YEAR>", current_year)
                with (case_dir / expected_temp_path).open("w", encoding="utf8") as f:
                    f.write(content)
        else:
            temp_paths = files
            expected_files = [f"{file}.expected" for file in files]
        subprocess.run([hawkeye, "format", "--fail-if-unknown", "--fail-if-updated=false", "--dry-run"], cwd=case_dir, check=True)

        for file in temp_paths:
            diff_files(case_dir / f"{file}.expected", case_dir / f"{file}.formatted")
    finally:
        # Remove all temp files at the end
        if create_temp_copy:
            for temp_path in temp_paths:
                if os.path.exists(case_dir / temp_path):
                    os.remove(case_dir / temp_path)
            for expected_file in expected_files:
                if os.path.exists(case_dir / expected_file):
                    os.remove(case_dir / expected_file)

def drive_no_change(name, files):
    """Run format --dry-run and assert the files are left untouched (no .formatted written)."""
    case_dir = basedir / name
    subprocess.run([hawkeye, "format", "--fail-if-unknown", "--fail-if-updated=false", "--dry-run"], cwd=case_dir, check=True)
    for file in files:
        formatted = case_dir / f"{file}.formatted"
        if formatted.exists():
            print(f"{name}: expected no change for {file}, but {formatted} was produced")
            formatted.unlink()
            exit(1)


def drive_expect_failure(name, args, expect_stderr=None):
    """Run hawkeye with args and assert a non-zero exit (e.g. existingStrategy = error).
    If expect_stderr is given, also assert that text appears on stderr."""
    case_dir = basedir / name
    result = subprocess.run([hawkeye, *args], cwd=case_dir, check=False, capture_output=expect_stderr is not None, text=True)
    if result.returncode == 0:
        print(f"{name}: expected non-zero exit from {args}, got 0")
        exit(1)
    if expect_stderr is not None and expect_stderr not in result.stderr:
        print(f"{name}: expected stderr to contain {expect_stderr!r}, got:\n{result.stderr}")
        exit(1)


def drive_expect_failure_no_change(name, files):
    """Run format --dry-run and assert BOTH a non-zero exit AND that no .formatted is written
    (a controlled failure that must not stack a duplicate header). Covers the unlisted-style
    foreign path and the stray-keyword path."""
    case_dir = basedir / name
    result = subprocess.run([hawkeye, "format", "--fail-if-unknown", "--dry-run"], cwd=case_dir, check=False)
    failed = False
    if result.returncode == 0:
        print(f"{name}: expected non-zero exit from format, got 0")
        failed = True
    for file in files:
        formatted = case_dir / f"{file}.formatted"
        if formatted.exists():
            print(f"{name}: expected no change for {file}, but {formatted} was produced (duplicate header?)")
            formatted.unlink()
            failed = True
    if failed:
        exit(1)


def drive_in_place(name, files):
    """Exercise the non-dry-run atomic save path: copy the fixture (source + config) into a
    throwaway temp dir, run `format` WITHOUT --dry-run so the file is rewritten in place via
    temp+rename, then diff the rewritten file against its .expected. Hermetic: never dirties
    the repo tree."""
    case_dir = basedir / name
    with tempfile.TemporaryDirectory(prefix=f"hawkeye-it-{name}-") as tmp:
        tmp = Path(tmp)
        shutil.copy2(case_dir / "licenserc.toml", tmp / "licenserc.toml")
        for file in files:
            shutil.copy2(case_dir / file, tmp / file)
        subprocess.run([hawkeye, "format", "--fail-if-unknown", "--fail-if-updated=false"], cwd=tmp, check=True)
        for file in files:
            diff_files(case_dir / f"{file}.expected", tmp / file)


drive("attrs_and_props", ["main.rs"])
drive("load_header_path", ["main.rs"])
drive("bom_issue", ["headless_bom.cs"])
drive("regression_blank_line", ["main.rs"])
drive("regression_no_blank_lines", ["repro.py"])
drive("disk_file_created_year", ["main.rs"], True)
drive("existing_foreign_style_replace", ["main.rs"])
drive("existing_foreign_style_replace_mixed", ["main.rs"])
drive_no_change("existing_foreign_style_skip", ["main.rs"])
drive_expect_failure("existing_foreign_style_error", ["format", "--dry-run"])
# A header in an unlisted comment style cannot be normalized: controlled failure, no duplicate.
drive_expect_failure_no_change("existing_foreign_style_unlisted", ["main.rs"])
# Form-1 residual dup (#210): a listed-style block ABOVE an unlisted-style notice. Stripping the
# listed block leaves the unlisted notice; format must re-detect it and fail, not stack on top.
drive_expect_failure_no_change("existing_foreign_style_replace_mixed_unlisted", ["main.rs"])
# A stray whole-word keyword in code looks licensed: controlled failure, no header inserted.
drive_expect_failure_no_change("keyword_in_code", ["main.rs"])
# Word-boundary fix: `copyrightHolder` is not a notice, so the header is inserted normally.
drive("keyword_identifier_no_header", ["main.rs"])
# 6.x [mapping]/useDefaultMapping config is rejected with a migration hint.
drive_expect_failure("legacy_mapping_config", ["format", "--dry-run"], expect_stderr="useDefaultHeaders")
# Non-dry-run in-place atomic write path.
drive_in_place("atomic_inplace", ["main.rs"])
