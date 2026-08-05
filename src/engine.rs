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
use std::path::Path;
use std::path::PathBuf;

use minijinja::Environment;
use minijinja::UndefinedBehavior;

use crate::Analyzer;
use crate::Config;
use crate::Edit;
use crate::FileOutcome;
use crate::HeaderSource;
use crate::Mode;
use crate::Report;
use crate::ResolvedConfig;
use crate::Result;
use crate::Status;
use crate::discovery::Discovery;
use crate::fs::PreparedWrite;
use crate::fs::io_error;

/// Filesystem orchestration over a resolved configuration and loaded header body.
pub struct Engine {
    root: PathBuf,
    config: ResolvedConfig,
    header_body: String,
    omitted: Vec<PathBuf>,
}

/// A repository-wide operation computed without mutating source files.
pub struct Plan {
    mode: Mode,
    root: PathBuf,
    files: Vec<PlannedFile>,
}

/// One file in a repository-wide plan.
pub struct PlannedFile {
    path: PathBuf,
    source_path: PathBuf,
    status: Status,
    original: Option<String>,
    edit: Option<Edit>,
}

impl Engine {
    /// Loads and resolves a `hawkeye.toml` file and its header source.
    pub fn load(config_path: impl AsRef<Path>) -> Result<Self> {
        let config_path = canonicalize(config_path.as_ref(), "resolve configuration path")?;
        let config_dir = config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let raw = fs::read_to_string(&config_path)
            .map_err(|source| io_error("read configuration from", &config_path, source))?;
        let config = Config::from_toml(&raw)?.resolve(&config_dir)?;

        let mut omitted = vec![config_path];
        let header_body = match config.header().source() {
            HeaderSource::Inline(text) => text.clone(),
            HeaderSource::File(path) => {
                let path = canonicalize(path, "resolve header path")?;
                let body = fs::read_to_string(&path)
                    .map_err(|source| io_error("read header from", &path, source))?;
                omitted.push(path);
                body
            }
        };
        let header_body = render_header(&config, &header_body)?;

        Ok(Self {
            root: config_dir,
            config,
            header_body,
            omitted,
        })
    }

    /// Creates an engine from already resolved parts.
    pub fn new(
        root: impl AsRef<Path>,
        config: ResolvedConfig,
        header_body: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            root: canonicalize(root.as_ref(), "resolve project root")?,
            header_body: render_header(&config, &header_body.into())?,
            config,
            omitted: Vec::new(),
        })
    }

    /// Returns the absolute project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the resolved configuration.
    pub fn config(&self) -> &ResolvedConfig {
        &self.config
    }

    /// Analyzes every selected file and returns a non-mutating repository plan.
    pub fn plan(&self, mode: Mode) -> Result<Plan> {
        let analyzer = Analyzer::new(&self.config, self.header_body.clone())?;
        let discovery =
            Discovery::new(self.root.clone(), self.config.files(), self.omitted.clone())?;
        let mut files = Vec::new();

        for source_path in discovery.paths()? {
            let path = source_path
                .strip_prefix(&self.root)
                .expect("discovery only returns paths below its root")
                .to_path_buf();
            let bytes =
                fs::read(&source_path).map_err(|source| io_error("read", &source_path, source))?;
            let Ok(original) = String::from_utf8(bytes) else {
                files.push(PlannedFile {
                    path,
                    source_path,
                    status: Status::Unsupported,
                    original: None,
                    edit: None,
                });
                continue;
            };

            let (status, edit) = analyzer.plan(&path, &original, mode)?.into_parts();
            files.push(PlannedFile {
                path,
                source_path,
                status,
                original: Some(original),
                edit,
            });
        }

        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Plan {
            mode,
            root: self.root.clone(),
            files,
        })
    }
}

impl Plan {
    /// Returns the operation represented by this plan.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Returns the absolute project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns every selected file in deterministic path order.
    pub fn files(&self) -> &[PlannedFile] {
        &self.files
    }

    /// Returns a serializable report without applying edits.
    pub fn report(&self) -> Report {
        Report::new(
            self.files
                .iter()
                .map(|file| FileOutcome::new(&file.path, file.status))
                .collect(),
        )
    }

    /// Preflights every changed file and then atomically replaces each one.
    pub fn apply(self) -> Result<Report> {
        let report = self.report();
        let mut prepared = Vec::new();

        for file in &self.files {
            let Some(edit) = &file.edit else {
                continue;
            };
            let original = file
                .original
                .as_deref()
                .expect("planned edits only exist for UTF-8 inputs");
            let content = edit.apply(original)?;
            prepared.push(PreparedWrite::prepare(
                &file.source_path,
                original,
                content,
            )?);
        }

        for write in prepared {
            write.commit()?;
        }
        Ok(report)
    }
}

impl PlannedFile {
    /// Returns the path relative to the project root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the analysis status.
    pub fn status(&self) -> Status {
        self.status
    }

    /// Returns the safe edit planned for this operation.
    pub fn edit(&self) -> Option<&Edit> {
        self.edit.as_ref()
    }

    /// Returns the analyzed UTF-8 source, or `None` for unsupported encoding.
    pub fn original(&self) -> Option<&str> {
        self.original.as_deref()
    }

    /// Computes the post-edit content without touching the filesystem.
    pub fn updated(&self) -> Result<Option<String>> {
        match (&self.original, &self.edit) {
            (Some(original), Some(edit)) => edit.apply(original).map(Some),
            (Some(original), None) => Ok(Some(original.clone())),
            (None, None) => Ok(None),
            (None, Some(_)) => unreachable!("unsupported files never carry edits"),
        }
    }
}

fn canonicalize(path: &Path, operation: &'static str) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|source| io_error(operation, path, source))
}

fn render_header(config: &ResolvedConfig, template: &str) -> Result<String> {
    let mut environment = Environment::new();
    environment.set_undefined_behavior(UndefinedBehavior::Strict);
    let rendered = environment
        .render_str(template, config.variables())
        .map_err(crate::Error::from)?;
    if rendered.trim().is_empty() {
        return Err(crate::Error::InvalidConfig(
            "rendered header text cannot be empty".to_owned(),
        ));
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_project() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(
            directory.path().join("hawkeye.toml"),
            r#"
[header]
text = "Copyright {{ year }} FastLabs Developers"
identifiers = ["Copyright"]

[variables]
year = 2026

[files]
use_gitignore = true

[[rules]]
patterns = ["**/*.rs"]
write_style = "slash"
read_styles = ["slash_star"]
"#,
        )
        .unwrap();
        fs::write(directory.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        directory
    }

    #[test]
    fn engine_plans_then_applies_repository_edits() {
        let directory = write_project();
        let engine = Engine::load(directory.path().join("hawkeye.toml")).unwrap();
        let plan = engine.plan(Mode::Format).unwrap();
        assert_eq!(plan.files().len(), 1);
        assert_eq!(plan.files()[0].path(), Path::new("src/main.rs"));
        assert_eq!(plan.files()[0].status(), Status::Missing);

        let report = plan.apply().unwrap();
        assert_eq!(report.files().len(), 1);
        assert_eq!(
            fs::read_to_string(directory.path().join("src/main.rs")).unwrap(),
            "// Copyright 2026 FastLabs Developers\n\nfn main() {}\n"
        );
    }

    #[test]
    fn apply_rejects_a_file_changed_after_planning() {
        let directory = write_project();
        let path = directory.path().join("src/main.rs");
        let canonical_path = fs::canonicalize(&path).unwrap();
        let engine = Engine::load(directory.path().join("hawkeye.toml")).unwrap();
        let plan = engine.plan(Mode::Format).unwrap();
        fs::write(&path, "changed\n").unwrap();

        assert!(
            matches!(plan.apply(), Err(crate::Error::StaleFile(file)) if file == canonical_path)
        );
        assert_eq!(fs::read_to_string(path).unwrap(), "changed\n");
    }

    #[test]
    fn invalid_utf8_is_reported_without_an_edit() {
        let directory = write_project();
        let path = directory.path().join("src/binary.rs");
        fs::write(&path, [0xff, 0xfe]).unwrap();
        let engine = Engine::load(directory.path().join("hawkeye.toml")).unwrap();
        let plan = engine.plan(Mode::Format).unwrap();
        let file = plan
            .files()
            .iter()
            .find(|file| file.path() == Path::new("src/binary.rs"))
            .unwrap();

        assert_eq!(file.status(), Status::Unsupported);
        assert!(file.edit().is_none());
    }

    #[test]
    fn external_header_and_configuration_are_not_analyzed() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::create_dir(directory.path().join("headers")).unwrap();
        fs::write(
            directory.path().join("headers/license.txt"),
            "Copyright 2026 FastLabs Developers\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("hawkeye.toml"),
            r#"
[header]
path = "headers/license.txt"
identifiers = ["Copyright"]

[[rules]]
patterns = ["**/*"]
write_style = "hash"
"#,
        )
        .unwrap();
        fs::write(directory.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let engine = Engine::load(directory.path().join("hawkeye.toml")).unwrap();
        let plan = engine.plan(Mode::Format).unwrap();
        let paths = plan
            .files()
            .iter()
            .map(PlannedFile::path)
            .collect::<Vec<_>>();

        assert_eq!(paths, [Path::new("src/main.rs")]);
    }

    #[test]
    fn custom_style_is_applied_end_to_end() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("hawkeye.toml"),
            r#"
use_default_rules = false

[header]
text = "Copyright 2026 FastLabs Developers"
identifiers = ["Copyright"]

[styles.semicolon]
kind = "line"
prefix = ";; "

[[rules]]
patterns = ["*.lisp"]
write_style = "semicolon"
"#,
        )
        .unwrap();
        let source = directory.path().join("main.lisp");
        fs::write(&source, "(print \"hello\")\n").unwrap();

        Engine::load(directory.path().join("hawkeye.toml"))
            .unwrap()
            .plan(Mode::Format)
            .unwrap()
            .apply()
            .unwrap();

        assert_eq!(
            fs::read_to_string(source).unwrap(),
            ";; Copyright 2026 FastLabs Developers\n\n(print \"hello\")\n"
        );
    }

    #[test]
    fn missing_template_variable_fails_before_discovery() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("hawkeye.toml"),
            r#"
[header]
text = "Copyright {{ missing_year }} FastLabs Developers"
identifiers = ["Copyright"]
"#,
        )
        .unwrap();

        assert!(matches!(
            Engine::load(directory.path().join("hawkeye.toml")),
            Err(crate::Error::Template(_))
        ));
    }

    #[test]
    fn empty_rendered_header_fails_during_engine_loading() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("hawkeye.toml"),
            r#"
[header]
text = "{{ value }}"
identifiers = ["Copyright"]

[variables]
value = ""
"#,
        )
        .unwrap();

        assert!(matches!(
            Engine::load(directory.path().join("hawkeye.toml")),
            Err(crate::Error::InvalidConfig(message))
                if message == "rendered header text cannot be empty"
        ));
    }
}
