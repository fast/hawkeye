// Copyright 2024 tison <wander4096@gmail.com>
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

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use exn::bail;
use exn::ensure;
use exn::OptionExt;
use exn::Result;
use exn::ResultExt;

use crate::config::Config;
use crate::document::factory::DocumentFactory;
use crate::document::model::default_header_rules;
use crate::document::Document;
use crate::error::Error;
use crate::git;
use crate::header::matcher::HeaderMatcher;
use crate::header::model::default_headers;
use crate::header::model::deserialize_header_definitions;
use crate::header::model::HeaderDef;
use crate::license::bundled_headers;
use crate::license::HeaderSource;
use crate::selection::Selection;

/// Callback for processing the result of checking license headers.
pub trait Callback {
    /// Called when the header is unknown.
    fn on_unknown(&mut self, path: &Path);

    /// Called when the header is matched.
    fn on_matched(&mut self, header: &HeaderMatcher, document: Document) -> Result<(), Error>;

    /// Called when the header is not matched.
    fn on_not_matched(&mut self, header: &HeaderMatcher, document: Document) -> Result<(), Error>;
}

pub fn check_license_header<C: Callback>(
    run_config: PathBuf,
    callback: &mut C,
) -> Result<(), Error> {
    let config = {
        let name = run_config.display().to_string();
        let raw = fs::read_to_string(&run_config)
            .or_raise(|| Error::new(format!("cannot load config: {name}")))?;
        reject_legacy_config(&raw, &name)?;
        toml::from_str::<Config>(&raw)
            .or_raise(|| Error::new(format!("cannot parse config file: {name}")))?
    };

    let config_dir = run_config
        .parent()
        .ok_or_raise(|| Error::new("cannot get parent directory of config file"))?;

    let basedir = config.base_dir.clone();
    ensure!(
        basedir.is_dir(),
        Error::new(format!(
            "{} does not exist or is not a directory",
            basedir.display()
        ))
    );

    let git_context = git::discover(&basedir, config.git)?;

    let selected_files = {
        let selection = Selection::new(
            basedir,
            config.header_path.as_ref(),
            &config.includes,
            &config.excludes,
            config.use_default_excludes,
            git_context.clone(),
        );
        selection.select()?
    };

    let header_rules = {
        // User rules take precedence; defaults are appended so first-match wins downstream.
        let mut rules = config.headers.clone();
        if config.use_default_headers {
            rules.extend(default_header_rules());
        }
        rules
    };

    let definitions = {
        let mut defs = HashMap::new();
        for (k, v) in default_headers() {
            match defs.entry(k) {
                Entry::Occupied(mut ent) => {
                    log::warn!("Default header {} is overridden", ent.key());
                    ent.insert(v);
                }
                Entry::Vacant(ent) => {
                    ent.insert(v);
                }
            }
        }

        for additional_header in &config.additional_headers {
            let additional_defs = load_additional_headers(additional_header, &config, config_dir)?;
            for (k, v) in additional_defs {
                match defs.entry(k) {
                    Entry::Occupied(mut ent) => {
                        log::warn!("Additional header {} is overridden", ent.key());
                        ent.insert(v);
                    }
                    Entry::Vacant(ent) => {
                        ent.insert(v);
                    }
                }
            }
        }

        defs
    };

    let header_matcher = {
        let header_source = load_header_sources(&config, config_dir)?;
        HeaderMatcher::new(header_source.content)
    };

    let git_file_attrs = git::resolve_file_attrs(git_context)?;

    let document_factory = DocumentFactory::new(
        header_rules,
        definitions,
        config.properties,
        config.keywords,
        git_file_attrs,
    )?;

    for file in selected_files {
        let document = match document_factory.create_document(&file)? {
            Some(document) => document,
            None => {
                callback.on_unknown(&file);
                continue;
            }
        };

        if document.is_unsupported() {
            callback.on_unknown(&file);
        } else if document.header_matched(&header_matcher, config.strict_check)? {
            callback.on_matched(&header_matcher, document)?;
        } else {
            callback.on_not_matched(&header_matcher, document)?;
        }
    }

    Ok(())
}

/// HawkEye 7.0 replaced the `[mapping.STYLE]` / `useDefaultMapping` config with the
/// `[[headers]]` rule list. Detect the removed keys and fail with a migration hint instead
/// of the opaque `unknown field` error that `deny_unknown_fields` would otherwise produce.
fn reject_legacy_config(raw: &str, name: &str) -> Result<(), Error> {
    let Ok(toml::Value::Table(table)) = toml::from_str::<toml::Value>(raw) else {
        return Ok(()); // let the typed parse below surface the real syntax error
    };
    if table.contains_key("mapping") || table.contains_key("useDefaultMapping") {
        bail!(Error::new(format!(
            "config {name} uses the 6.x `[mapping]` / `useDefaultMapping` format, removed in 7.0.\n\
             Replace each `[mapping.STYLE] {{ extensions = [...], filenames = [...] }}` with:\n\
             \n  [[headers]]\n  extensions = [...]\n  filenames = [...]\n  styles = [\"STYLE\"]\n  existingStrategy = \"replace\"\n\
             \nand replace `useDefaultMapping` with `useDefaultHeaders`. See the README configuration section."
        )));
    }
    Ok(())
}

fn load_additional_headers(
    additional_header: impl AsRef<Path>,
    config: &Config,
    config_dir: &Path,
) -> Result<HashMap<String, HeaderDef>, Error> {
    fn make_error(path: &Path) -> String {
        format!("cannot load additional header {}", path.display())
    }

    let additional_header = additional_header.as_ref();

    // 1. Based on config directory.
    let path = {
        let mut path = config_dir.to_path_buf();
        path.push(additional_header);
        path
    };
    if let Ok(content) = fs::read_to_string(&path) {
        return deserialize_header_definitions(content).or_raise(|| Error::new(make_error(&path)));
    }

    // 2. Based on the base_dir.
    let path = {
        let mut path = config.base_dir.clone();
        path.push(additional_header);
        path
    };
    if let Ok(content) = fs::read_to_string(&path) {
        return deserialize_header_definitions(content).or_raise(|| Error::new(make_error(&path)));
    }

    // 3. Based on current working directory.
    let path = additional_header;
    if let Ok(content) = fs::read_to_string(path) {
        return deserialize_header_definitions(content).or_raise(|| Error::new(make_error(path)));
    }

    bail!(Error::new(format!(
        "cannot find header definitions: {}",
        additional_header.display()
    )))
}

fn load_header_sources(config: &Config, config_dir: &Path) -> Result<HeaderSource, Error> {
    // 1. inline_header takes priority.
    if let Some(content) = config.inline_header.as_ref().cloned() {
        return Ok(HeaderSource { content });
    }

    // 2. Then, try to load from header_path.
    let header_path = config.header_path.as_ref().ok_or_else(|| {
        Error::new("no header source found (both inline_header and header_path are None)")
    })?;

    // 2.1 Based on config directory.
    let path = {
        let mut path = config_dir.to_path_buf();
        path.push(header_path);
        path
    };
    if let Ok(content) = fs::read_to_string(path) {
        return Ok(HeaderSource { content });
    }

    // 2.2 Based on the base_dir.
    let path = {
        let mut path = config.base_dir.clone();
        path.push(header_path);
        path
    };
    if let Ok(content) = fs::read_to_string(path) {
        return Ok(HeaderSource { content });
    }

    // 2.3 Based on current working directory.
    if let Ok(content) = fs::read_to_string(header_path) {
        return Ok(HeaderSource { content });
    }

    // 3. Finally, fallback to try bundled headers.
    bundled_headers(header_path).ok_or_raise(|| {
        Error::new(format!(
            "no header source found (header_path is invalid: {header_path})"
        ))
    })
}
