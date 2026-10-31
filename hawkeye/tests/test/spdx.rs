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

use super::support::Project;
use super::support::assert_exit;
use super::support::assert_report;
use super::support::stderr;

fn configure(project: &Project, props: &str) {
    project.write(
        "licenserc.toml",
        format!("[header]\nbuiltin = \"SPDX\"\n\n[files]\nincludes = [\"*.rs\", \"*.c\"]\n\n[props]\n{props}\n"),
    );
}

#[test]
fn spdx_headers_round_trip_without_keywords() {
    for (props, header) in [
        (
            "spdx_license_expression = \"Apache-2.0 OR MIT\"",
            "// SPDX-License-Identifier: Apache-2.0 OR MIT\n\n",
        ),
        (
            "spdx_license_expression = \"GPL-3.0-only WITH Classpath-exception-2.0\"\ncopyright = \"2020-2026 Acme Developers <dev@example.com>\"",
            "// SPDX-FileCopyrightText: 2020-2026 Acme Developers <dev@example.com>\n//\n// SPDX-License-Identifier: GPL-3.0-only WITH Classpath-exception-2.0\n\n",
        ),
    ] {
        let project = Project::empty();
        configure(&project, props);
        let source = "// This comment belongs to the function.\nfn main() {}\n";
        project.write("source.rs", source);

        let checked = project.run(["check", "--output-format=json"]);
        assert_exit(&checked, 1);
        assert_report(&checked, &[("source.rs", "add")]);
        assert_eq!(project.read("source.rs"), source);

        assert_exit(&project.run(["format"]), 0);
        let expected = format!("{header}{source}");
        assert_eq!(project.read("source.rs"), expected);
        assert_exit(&project.run(["check"]), 0);
        let repeated = project.run(["format", "--output-format=json"]);
        assert_exit(&repeated, 0);
        assert_report(&repeated, &[("source.rs", "clean")]);
        assert_eq!(project.read("source.rs"), expected);
        assert_exit(&project.run(["remove"]), 0);
        assert_eq!(project.read("source.rs"), source);
    }
}

#[test]
fn spdx_license_and_copyright_can_be_updated() {
    let project = Project::empty();
    configure(&project, "spdx_license_expression = \"Apache-2.0\"");
    project.write("source.rs", "fn main() {}\n");
    assert_exit(&project.run(["format"]), 0);

    for (expression, copyright) in [
        ("MIT", None),
        ("MIT", Some("2025 Acme Developers")),
        ("Apache-2.0 OR MIT", Some("2025-2026 Acme Developers")),
    ] {
        let mut props = format!("spdx_license_expression = {expression:?}");
        let mut expected = String::new();
        if let Some(copyright) = copyright {
            props.push_str(&format!("\ncopyright = {copyright:?}"));
            expected.push_str(&format!("// SPDX-FileCopyrightText: {copyright}\n//\n"));
        }
        expected.push_str(&format!(
            "// SPDX-License-Identifier: {expression}\n\nfn main() {{}}\n"
        ));
        configure(&project, &props);
        let result = project.run(["format", "--output-format=json"]);
        assert_exit(&result, 0);
        assert_report(&result, &[("source.rs", "replace")]);
        assert_eq!(project.read("source.rs"), expected);
        assert_exit(&project.run(["check"]), 0);
    }
    assert_exit(&project.run(["remove"]), 0);
    assert_eq!(project.read("source.rs"), "fn main() {}\n");
}

#[test]
fn spdx_style_migration_preserves_line_endings() {
    let project = Project::empty();
    configure(
        &project,
        "spdx_license_expression = \"MIT\"\ncopyright = \"2026 Acme\"",
    );
    project.write(
        "licenserc.toml",
        format!("{}\n[[rules]]\nextensions = [\"c\"]\nstyle_out = \"slashstar\"\nstyles_in = [\"slashstar\", \"doubleslash\"]\n", project.read("licenserc.toml")),
    );
    project.write("source.c", "// SPDX-License-Identifier: Apache-2.0\r\n// SPDX-FileCopyrightText: 2025 Acme\r\n\r\nint main() {}\r\n");
    assert_exit(&project.run(["format"]), 0);
    assert_eq!(
        project.read("source.c"),
        "/*\r\n * SPDX-FileCopyrightText: 2026 Acme\r\n *\r\n * SPDX-License-Identifier: MIT\r\n */\r\n\r\nint main() {}\r\n"
    );
    assert_exit(&project.run(["check"]), 0);
    assert_exit(&project.run(["format"]), 0);
    assert_exit(&project.run(["remove"]), 0);
    assert_eq!(project.read("source.c"), "int main() {}\r\n");
}

#[test]
fn spdx_does_not_discard_unmanaged_text_or_copyrights() {
    let project = Project::empty();
    for props in [
        "spdx_license_expression = \"MIT\"",
        "spdx_license_expression = \"MIT\"\ncopyright = \"2026 Acme\"",
    ] {
        configure(&project, props);
        for source in [
            "// SPDX-License-Identifier: Apache-2.0\n// SAFETY: required by the code below.\nfn main() {}\n",
            "// Copyright 2020 Original Authors\n// SPDX-License-Identifier: Apache-2.0\n\nfn main() {}\n",
            "// SPDX-FileCopyrightText: 2020 First Author\n// SPDX-FileCopyrightText: 2021 Second Author\n// SPDX-License-Identifier: Apache-2.0\n\nfn main() {}\n",
            "// SPDX-License-Identifier: Apache-2.0\n// SPDX-License-Identifier: MIT\n\nfn main() {}\n",
            "// SPDX-License-Identifier:\n\nfn main() {}\n",
        ] {
            for command in ["format", "remove"] {
                project.write("source.rs", source);
                let result = project.run([command, "--output-format=json"]);
                assert_exit(&result, 1);
                assert_report(&result, &[("source.rs", "conflict")]);
                assert_eq!(project.read("source.rs"), source);
            }
        }
    }

    configure(&project, "spdx_license_expression = \"MIT\"");
    let source = "// SPDX-FileCopyrightText: 2020 Original Authors\n// SPDX-License-Identifier: Apache-2.0\n\nfn main() {}\n";
    for command in ["format", "remove"] {
        project.write("source.rs", source);
        assert_exit(&project.run([command]), 1);
        assert_eq!(project.read("source.rs"), source);
    }

    let source = "// Copyright 2020 Original Authors\n\nfn main() {}\n";
    project.write("source.rs", source);
    assert_exit(&project.run(["format"]), 0);
    assert_eq!(
        project.read("source.rs"),
        format!("// SPDX-License-Identifier: MIT\n\n{source}")
    );
    assert_exit(&project.run(["remove"]), 0);
    assert_eq!(project.read("source.rs"), source);
}

#[test]
fn spdx_invalid_props_fail_before_writing_files() {
    let project = Project::empty();
    for (props, field) in [
        ("", "props.spdx_license_expression"),
        (
            "spdx_license_expression = 123",
            "props.spdx_license_expression",
        ),
        (
            "spdx_license_expression = \" \"",
            "props.spdx_license_expression",
        ),
        (
            "spdx_license_expression = \"MIT\\nOR Apache-2.0\"",
            "props.spdx_license_expression",
        ),
        (
            "spdx_license_expression = \"MIT\"\ncopyright = []",
            "props.copyright",
        ),
        (
            "spdx_license_expression = \"MIT\"\ncopyright = \"\"",
            "props.copyright",
        ),
        (
            "spdx_license_expression = \"MIT\"\ncopyright = \"2026 Acme\\rOther\"",
            "props.copyright",
        ),
    ] {
        configure(&project, props);
        project.write("source.rs", "fn main() {}\n");
        let result = project.run(["format"]);
        assert_exit(&result, 2);
        assert!(stderr(&result).contains(field), "{}", stderr(&result));
        assert_eq!(project.read("source.rs"), "fn main() {}\n");
    }
}

#[test]
fn spdx_explicit_keywords_still_constrain_recognition() {
    let project = Project::empty();
    configure(&project, "spdx_license_expression = \"MIT\"");
    project.write(
        "licenserc.toml",
        project
            .read("licenserc.toml")
            .replace("[header]", "[header]\nkeywords = [\"copyright\"]"),
    );
    project.write("source.rs", "fn main() {}\n");
    let result = project.run(["format"]);
    assert_exit(&result, 2);
    assert!(stderr(&result).contains("recognition keyword \"copyright\""));
    assert_eq!(project.read("source.rs"), "fn main() {}\n");
}
