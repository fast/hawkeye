// Copyright 2026 FastLabs Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fs;

use super::support::Project;
use super::support::assert_exit;
use super::support::assert_report;

#[test]
fn mixed_repository_formats_checks_and_removes_headers() {
    let project = Project::from_case("mixed");
    let before_app = project.read("app.rs");
    let before_makefile = project.read("Makefile");
    let before_types = project.read("types.d.ts");

    let checked = project.run(["check", "--output-format=json"]);
    assert_exit(&checked, 1);
    assert_report(
        &checked,
        &[
            ("Makefile", "add"),
            ("app.rs", "add"),
            ("legacy.rs", "replace"),
            ("not_ignored_without_repository.rs", "add"),
            ("notes.txt", "unsupported"),
            ("types.d.ts", "add"),
        ],
    );

    let formatted = project.run(["format", "--output-format=json"]);
    assert_exit(&formatted, 0);
    assert_report(
        &formatted,
        &[
            ("Makefile", "add"),
            ("app.rs", "add"),
            ("legacy.rs", "replace"),
            ("not_ignored_without_repository.rs", "add"),
            ("notes.txt", "unsupported"),
            ("types.d.ts", "add"),
        ],
    );
    assert_eq!(
        project.read("app.rs"),
        "// Copyright 2026 Acme Labs\n// Sequence 1-2-3\n\nfn main() {}\n"
    );
    assert_eq!(
        project.read("legacy.rs"),
        "// Copyright 2026 Acme Labs\n// Sequence 1-2-3\n\nfn legacy() {}\n"
    );
    assert_eq!(project.read("excluded/skip.rs"), "fn excluded() {}\n");

    let checked = project.run(["check", "--output-format=json"]);
    assert_exit(&checked, 0);
    assert_report(
        &checked,
        &[
            ("Makefile", "clean"),
            ("app.rs", "clean"),
            ("legacy.rs", "clean"),
            ("not_ignored_without_repository.rs", "clean"),
            ("notes.txt", "unsupported"),
            ("types.d.ts", "clean"),
        ],
    );

    let idempotent = project.run(["format", "--output-format=json"]);
    assert_exit(&idempotent, 0);
    assert_report(
        &idempotent,
        &[
            ("Makefile", "clean"),
            ("app.rs", "clean"),
            ("legacy.rs", "clean"),
            ("not_ignored_without_repository.rs", "clean"),
            ("notes.txt", "unsupported"),
            ("types.d.ts", "clean"),
        ],
    );

    let removed = project.run(["remove", "--output-format=json"]);
    assert_exit(&removed, 0);
    assert_report(
        &removed,
        &[
            ("Makefile", "remove"),
            ("app.rs", "remove"),
            ("legacy.rs", "remove"),
            ("not_ignored_without_repository.rs", "remove"),
            ("notes.txt", "unsupported"),
            ("types.d.ts", "remove"),
        ],
    );
    assert_eq!(project.read("app.rs"), before_app);
    assert_eq!(project.read("Makefile"), before_makefile);
    assert_eq!(project.read("types.d.ts"), before_types);
    assert_eq!(project.read("legacy.rs"), "fn legacy() {}\n");
}

#[test]
fn format_preserves_preambles_and_existing_line_endings() {
    let project = Project::from_case("preambles");
    project.write("bom.cs", b"\xef\xbb\xbfpublic class Example {}\n");
    project.write("windows.rs", b"fn main() {}\r\n");
    project.write("mixed.rs", b"fn first() {}\r\nfn second() {}\n");
    project.write(
        "main.php",
        "<?php declare(strict_types=1);\n\necho \"hello\";\n",
    );

    let formatted = project.run(["format", "--output-format=json"]);
    assert_exit(&formatted, 0);
    assert_report(
        &formatted,
        &[
            ("bom.cs", "add"),
            ("document.xml", "add"),
            ("document.yaml", "add"),
            ("main.php", "add"),
            ("mixed.rs", "add"),
            ("script.py", "add"),
            ("windows.rs", "add"),
        ],
    );
    assert_eq!(
        project.read_bytes("bom.cs"),
        b"\xef\xbb\xbf/*\n * Copyright 2026 Acme Labs\n */\n\npublic class Example {}\n"
    );
    assert_eq!(
        project.read("document.xml"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!--\nCopyright 2026 Acme Labs\n-->\n\n<project />\n"
    );
    assert_eq!(
        project.read("document.yaml"),
        "%YAML 1.2\n%TAG !example! tag:example.com,2026:\n# Copyright 2026 Acme Labs\n\n---\nvalue: !example!value content\n"
    );
    assert_eq!(
        project.read("script.py"),
        "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\n# Copyright 2026 Acme Labs\n\nprint(\"hello\")\n"
    );
    assert_eq!(
        project.read_bytes("windows.rs"),
        b"// Copyright 2026 Acme Labs\r\n\r\nfn main() {}\r\n"
    );
    assert_eq!(
        project.read_bytes("mixed.rs"),
        b"// Copyright 2026 Acme Labs\r\n\r\nfn first() {}\r\nfn second() {}\n"
    );
    assert_eq!(
        project.read("main.php"),
        "<?php declare(strict_types=1);\n/*\n * Copyright 2026 Acme Labs\n */\n\necho \"hello\";\n"
    );
    assert_exit(&project.run(["check"]), 0);
}

#[test]
fn conventional_block_comment_layouts_are_recognized() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
builtin = "Apache-2.0-ASF"

[files]
includes = ["**/*.java", "**/*.xml"]
"#,
    );
    project.write(
        "Example.java",
        r#"/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 */

class Example {}
"#,
    );
    project.write(
        "pom.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!--
Licensed to the Apache Software Foundation (ASF) under one
or more contributor license agreements.  See the NOTICE file
distributed with this work for additional information
regarding copyright ownership.  The ASF licenses this file
to you under the Apache License, Version 2.0 (the
"License"); you may not use this file except in compliance
with the License.  You may obtain a copy of the License at

  http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing,
software distributed under the License is distributed on an
"AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
KIND, either express or implied.  See the License for the
specific language governing permissions and limitations
under the License.
-->

<project />
"#,
    );

    let checked = project.run(["check", "--output-format=json"]);
    assert_exit(&checked, 1);
    assert_report(
        &checked,
        &[("Example.java", "replace"), ("pom.xml", "clean")],
    );

    let formatted = project.run(["format", "--output-format=json"]);
    assert_exit(&formatted, 0);
    assert_report(
        &formatted,
        &[("Example.java", "replace"), ("pom.xml", "clean")],
    );
    assert_exit(&project.run(["check"]), 0);
}

#[test]
fn custom_line_and_block_styles_round_trip() {
    let project = Project::from_case("styles");

    let formatted = project.run(["format", "--output-format=json"]);
    assert_exit(&formatted, 0);
    assert_report(
        &formatted,
        &[("example.block", "replace"), ("example.line", "replace")],
    );
    assert_eq!(
        project.read("example.line"),
        "<!-- Copyright 2026 Acme    -->\n<!-- Licensed under Example -->\n\nline content\n"
    );
    assert_eq!(
        project.read("example.block"),
        "<*\n  Copyright 2026 Acme\n  Licensed under Example\n*>\n\nblock content\n"
    );
    assert_exit(&project.run(["check"]), 0);
    assert_exit(&project.run(["format"]), 0);
}

#[test]
fn ambiguous_or_partial_headers_are_never_edited() {
    let project = Project::from_case("conflict");
    let foreign = project.read("foreign.rs");

    let formatted = project.run(["format", "--output-format=json"]);
    assert_exit(&formatted, 1);
    assert_report(
        &formatted,
        &[("foreign.rs", "conflict"), ("ordinary.rs", "add")],
    );
    assert_eq!(project.read("foreign.rs"), foreign);
    assert_eq!(
        project.read("ordinary.rs"),
        "// Confidential © Siemens 2026\n\n// An ordinary leading comment.\n\nfn ordinary_comment() {}\n"
    );

    let unsafe_project = Project::empty();
    unsafe_project.write(
        "licenserc.toml",
        r#"[header]
text = """
Copyright 2026 Acme
Licensed under Example
"""

[files]
includes = ["**/*.rs"]
"#,
    );
    for source in [
        "// Copyright 2025 Acme\n\n// Licensed under Example\n\nfn main() {}\n",
        "// Copyright 2025 Acme\n// SAFETY: this comment belongs to the code.\nfn main() {}\n",
        "/*\n * Copyright 2025 Acme\n * SAFETY: this comment belongs to the code.\n */\nfn main() {}\n",
    ] {
        for command in ["format", "remove"] {
            unsafe_project.write("main.rs", source);
            let result = unsafe_project.run([command, "--output-format=json"]);
            assert_exit(&result, 1);
            assert_report(&result, &[("main.rs", "conflict")]);
            assert_eq!(unsafe_project.read("main.rs"), source);
        }
    }
}

#[test]
fn header_template_files_are_not_selected_as_sources() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
path = "license.rs"

[files]
includes = ["**/*.rs"]
"#,
    );
    project.write("license.rs", "Copyright 2026 Acme\n");
    fs::hard_link(
        project.path().join("license.rs"),
        project.path().join("license-hardlink.rs"),
    )
    .expect("create header template hard link");
    #[cfg(unix)]
    std::os::unix::fs::symlink("license.rs", project.path().join("license-link.rs"))
        .expect("create header template symlink");
    project.write("main.rs", "fn main() {}\n");

    let formatted = project.run(["format", "--output-format=json"]);
    assert_exit(&formatted, 0);
    assert_report(&formatted, &[("main.rs", "add")]);
    assert_eq!(project.read("license.rs"), "Copyright 2026 Acme\n");
}

#[test]
fn format_writes_through_hard_links() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["source.rs"]

[git]
ignore = "disable"
"#,
    );
    project.write("target.txt", "fn main() {}\n");
    fs::hard_link(
        project.path().join("target.txt"),
        project.path().join("source.rs"),
    )
    .expect("create source hard link");

    assert_exit(&project.run(["format"]), 0);
    assert_eq!(
        project.read("target.txt"),
        "// Copyright 2026 Acme\n\nfn main() {}\n"
    );
    assert_eq!(project.read("source.rs"), project.read("target.txt"));
}

#[cfg(unix)]
#[test]
fn format_writes_through_file_symlinks() {
    for git_ignore in ["disable", "auto"] {
        let project = Project::empty();
        if git_ignore == "auto" {
            project.git(["init", "-b", "main"]);
        }
        project.write(
            "licenserc.toml",
            format!(
                r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["source.rs"]

[git]
ignore = "{git_ignore}"
"#
            ),
        );
        project.write("target.txt", "fn main() {}\n");
        std::os::unix::fs::symlink("target.txt", project.path().join("source.rs"))
            .expect("create source symlink");

        assert_exit(&project.run(["format"]), 0);
        assert!(
            fs::symlink_metadata(project.path().join("source.rs"))
                .expect("read source metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            project.read("target.txt"),
            "// Copyright 2026 Acme\n\nfn main() {}\n"
        );
    }
}
