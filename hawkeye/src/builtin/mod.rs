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

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::config::RuleConfig;
use crate::config::StyleConfig;

pub static HEADERS: LazyLock<BTreeMap<&'static str, &'static str>> = LazyLock::new(|| {
    BTreeMap::from([
        ("Apache-2.0", include_str!("headers/Apache-2.0.txt")),
        ("Apache-2.0-ASF", include_str!("headers/Apache-2.0-ASF.txt")),
        ("Elastic-2.0", include_str!("headers/Elastic-2.0.txt")),
    ])
});

pub static RULES: LazyLock<Vec<RuleConfig>> = LazyLock::new(|| {
    #[derive(Deserialize)]
    struct Rules {
        rules: Vec<RuleConfig>,
    }

    let rules = toml::from_str::<Rules>(include_str!("rules.toml")).unwrap();
    rules.rules
});

pub static STYLES: LazyLock<BTreeMap<String, StyleConfig>> = LazyLock::new(|| {
    #[derive(Deserialize)]
    struct Styles {
        styles: BTreeMap<String, StyleConfig>,
    }

    let styles = toml::from_str::<Styles>(include_str!("styles.toml")).unwrap();
    styles.styles
});
