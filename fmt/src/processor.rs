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
use crate::document::model::default_mapping;
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

/// Check the license headers of all the files selected by the config.
pub fn check_license_header<C: Callback>(
    run_config: PathBuf,
    callback: &mut C,
) -> Result<(), Error> {
    do_check_license_header(run_config, None, callback)
}

/// Check the license headers of the given files and directories only.
///
/// Paths are resolved against the `baseDir` of the config, and they are still subject to its
/// includes and excludes. This skips walking the whole `baseDir`, as well as the parts of the Git
/// history that are irrelevant to these paths, which is significantly faster on large
/// repositories. An empty `paths` selects no file at all.
pub fn check_license_header_of_paths<C: Callback>(
    run_config: PathBuf,
    paths: &[PathBuf],
    callback: &mut C,
) -> Result<(), Error> {
    do_check_license_header(run_config, Some(paths), callback)
}

fn do_check_license_header<C: Callback>(
    run_config: PathBuf,
    paths: Option<&[PathBuf]>,
    callback: &mut C,
) -> Result<(), Error> {
    let config = {
        let name = run_config.display().to_string();
        let config = fs::read_to_string(&run_config)
            .or_raise(|| Error::new(format!("cannot load config: {name}")))?;
        toml::from_str::<Config>(&config)
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
        match paths {
            None => selection.select()?,
            Some(paths) => selection.select_paths(paths)?,
        }
    };

    if paths.is_some() && selected_files.is_empty() {
        log::info!("no file is selected to process");
        return Ok(());
    }

    let mapping = {
        let mut mapping = config.mapping.clone();
        if config.use_default_mapping {
            let default_mapping = default_mapping();
            for m in default_mapping {
                if let Some(o) = mapping.get(&m) {
                    log::warn!("default mapping {m:?} is override by {o:?}");
                    continue;
                }
                mapping.insert(m);
            }
        }
        mapping
    };

    let definitions = {
        let mut defs = HashMap::new();
        for (k, v) in default_headers() {
            match defs.entry(k) {
                Entry::Occupied(mut ent) => {
                    log::warn!("Default header {} is override", ent.key());
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
                        log::warn!("Additional header {} is override", ent.key());
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

    let git_file_attrs = match paths {
        None => git::resolve_file_attrs(git_context)?,
        Some(_) => {
            let interests = selected_files.iter().cloned().collect();
            git::resolve_file_attrs_of(git_context, Some(&interests))?
        }
    };

    let document_factory = DocumentFactory::new(
        mapping,
        definitions,
        config.properties,
        config.keywords,
        git_file_attrs,
    );

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
