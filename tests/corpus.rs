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

use hawkeye::Analyzer;
use hawkeye::Config;
use hawkeye::Mode;
use hawkeye::Status;

const HEADER: &str = concat!(
    "Licensed to the Apache Software Foundation (ASF) under one\n",
    "or more contributor license agreements."
);

#[test]
fn representative_downstream_corpus_is_normalized_idempotently() {
    let config = Config::from_toml(
        r#"
[header]
text = """Licensed to the Apache Software Foundation (ASF) under one
or more contributor license agreements."""
identifiers = ["Apache Software Foundation", "contributor license agreements"]
"#,
    )
    .unwrap()
    .resolve(".")
    .unwrap();
    let analyzer = Analyzer::new(&config, HEADER).unwrap();
    let cases = [
        (
            "src/lib.rs",
            concat!(
                "/*\n",
                " * Licensed to the Apache Software Foundation (ASF) under one\n",
                " * or more contributor license agreements.\n",
                " */\n\n",
                "fn main() {}\n"
            ),
            Status::Replaceable,
            "// Licensed to the Apache Software Foundation",
        ),
        (
            "tools/release.py",
            "#!/usr/bin/env python3\nprint('release')\n",
            Status::Missing,
            "#!/usr/bin/env python3\n# Licensed to the Apache Software Foundation",
        ),
        (
            "pom.xml",
            "<?xml version=\"1.0\"?>\n<project/>\n",
            Status::Missing,
            "<?xml version=\"1.0\"?>\n<!--\nLicensed to the Apache Software Foundation",
        ),
    ];

    for (path, input, expected_status, expected_prefix) in cases {
        let first = analyzer.plan(path, input, Mode::Format).unwrap();
        let output = first.apply(input).unwrap();
        let second = analyzer.plan(path, &output, Mode::Format).unwrap();

        assert_eq!(first.status(), expected_status, "{path}");
        assert!(output.starts_with(expected_prefix), "{path}: {output}");
        assert_eq!(output.matches("Apache Software Foundation").count(), 1);
        assert_eq!(second.status(), Status::Clean, "{path}");
        assert_eq!(second.apply(&output).unwrap(), output, "{path}");
    }
}
