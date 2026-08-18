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

use std::borrow::Cow;
use std::collections::BTreeMap;

use minijinja::AutoEscape;
use minijinja::Environment;
use minijinja::UndefinedBehavior;

use crate::Error;
use crate::ErrorKind;
use crate::engine::FileAttrs;

pub struct HeaderTemplate {
    environment: Environment<'static>,
}

impl HeaderTemplate {
    pub fn new<S: Into<Cow<'static, str>>>(source: S) -> Result<Self, Error> {
        let mut environment = Environment::new();
        environment.set_undefined_behavior(UndefinedBehavior::Strict);
        environment.set_auto_escape_callback(|_| AutoEscape::None);
        environment
            .add_template_owned("header", source)
            .map_err(|err| {
                Error::new(ErrorKind::ConfigInvalid, "cannot compile header template")
                    .with_source(err)
            })?;
        Ok(Self { environment })
    }

    pub fn render(
        &self,
        props: &BTreeMap<String, toml::Value>,
        attrs: &FileAttrs,
    ) -> Result<String, Error> {
        let template = self.environment.get_template("header").unwrap();
        let rendered = template
            .render(minijinja::context! { props, attrs })
            .map_err(|err| {
                Error::new(ErrorKind::ConfigInvalid, "cannot render header template")
                    .with_source(err)
            })?;
        let normalized = rendered.replace("\r\n", "\n").replace('\r', "\n");
        let normalized = normalized.trim_matches('\n').to_owned();
        if normalized.trim().is_empty() {
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                "header template rendered an empty value",
            ));
        }
        if normalized.contains('\0') {
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                "header template rendered a NUL byte",
            ));
        }
        Ok(normalized)
    }
}
