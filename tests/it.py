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

def drive(name, files, create_temp_copy=False, extra_args=None, untouched=None):
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
        command = [hawkeye, "format", "--fail-if-unknown", "--fail-if-updated=false", "--dry-run"]
        subprocess.run(command + (extra_args or []), cwd=case_dir, check=True)

        for file in temp_paths:
            diff_files(case_dir / f"{file}.expected", case_dir / f"{file}.formatted")
        for file in untouched or []:
            if os.path.exists(case_dir / f"{file}.formatted"):
                print(f"{file} should not have been processed")
                exit(1)
    finally:
        # Remove all temp files at the end
        if create_temp_copy:
            for temp_path in temp_paths:
                if os.path.exists(case_dir / temp_path):
                    os.remove(case_dir / temp_path)
            for expected_file in expected_files:
                if os.path.exists(case_dir / expected_file):
                    os.remove(case_dir / expected_file)

drive("attrs_and_props", ["main.rs"])
drive("load_header_path", ["main.rs"])
drive("bom_issue", ["headless_bom.cs"])
drive("regression_blank_line", ["main.rs"])
drive("regression_no_blank_lines", ["repro.py"])
drive("disk_file_created_year", ["main.rs"], True)
drive("select_paths", ["selected.rs"], extra_args=["selected.rs"], untouched=["not_selected.rs"])
drive("select_paths", ["selected.rs"], extra_args=["--files-from", "files.txt"], untouched=["not_selected.rs"])
