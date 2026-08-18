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

use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use serde::Deserialize;
use tempfile::TempDir;

pub struct Project {
    #[expect(dead_code, reason = "keeps the temporary directory alive")]
    temporary: TempDir,
    root: PathBuf,
    git_config: PathBuf,
}

impl Project {
    pub fn empty() -> Self {
        Self::with_name("worktree")
    }

    #[cfg(target_os = "linux")]
    pub fn named(name: impl AsRef<Path>) -> Self {
        Self::with_name(name)
    }

    fn with_name(name: impl AsRef<Path>) -> Self {
        let temporary = tempfile::tempdir().expect("create temporary project");
        let root = temporary.path().join(name);
        fs::create_dir(&root).expect("create project root");
        // Host-level aliases, excludes, signing, and identity must not change fixture behavior.
        let git_config = temporary.path().join("gitconfig");
        fs::write(&git_config, "").expect("create isolated Git config");
        Self {
            temporary,
            root,
            git_config,
        }
    }

    pub fn from_case(name: &str) -> Self {
        let project = Self::empty();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("cases")
            .join(name);
        copy_tree(&source, &project.root);
        project
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn write(&self, path: impl AsRef<Path>, content: impl AsRef<[u8]>) {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create source parent");
        }
        fs::write(path, content).expect("write project file");
    }

    pub fn read(&self, path: impl AsRef<Path>) -> String {
        fs::read_to_string(self.root.join(path)).expect("read UTF-8 project file")
    }

    pub fn read_bytes(&self, path: impl AsRef<Path>) -> Vec<u8> {
        fs::read(self.root.join(path)).expect("read project file")
    }

    pub fn command<I, S>(&self, arguments: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(env!("CARGO_BIN_EXE_hawkeye"));
        command
            .args(arguments)
            .current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.git_config);
        command
    }

    pub fn run<I, S>(&self, arguments: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command(arguments).output().expect("run hawkeye")
    }

    pub fn run_with_stdin<I, S>(&self, arguments: I, input: impl AsRef<[u8]>) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut child = self
            .command(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start hawkeye");
        child
            .stdin
            .take()
            .expect("open hawkeye stdin")
            .write_all(input.as_ref())
            .expect("write hawkeye stdin");
        child.wait_with_output().expect("wait for hawkeye")
    }

    pub fn git<I, S>(&self, arguments: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self
            .git_command()
            .args(arguments)
            .output()
            .expect("run Git");
        assert!(
            output.status.success(),
            "Git failed\nstdout:\n{}\nstderr:\n{}",
            stdout(&output),
            stderr(&output)
        );
    }

    pub fn git_stdout<I, S>(&self, arguments: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self
            .git_command()
            .args(arguments)
            .output()
            .expect("run Git");
        assert!(
            output.status.success(),
            "Git failed\nstdout:\n{}\nstderr:\n{}",
            stdout(&output),
            stderr(&output)
        );
        stdout(&output).trim().to_owned()
    }

    pub fn commit(&self, message: &str, author: &str, email: &str, date: &str) {
        let output = self
            .git_command()
            .args(["commit", "-m", message])
            .env("GIT_AUTHOR_NAME", author)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_NAME", author)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .expect("commit Git changes");
        assert!(
            output.status.success(),
            "Git commit failed\nstdout:\n{}\nstderr:\n{}",
            stdout(&output),
            stderr(&output)
        );
    }

    fn git_command(&self) -> Command {
        let mut command = Command::new("git");
        command
            .args(["-c", "commit.gpgsign=false"])
            .current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.git_config);
        command
    }
}

#[derive(Debug, Deserialize)]
pub struct JsonReport {
    pub files: Vec<JsonFile>,
}

#[derive(Debug, Deserialize)]
pub struct JsonFile {
    pub path: PathBuf,
    pub outcome: String,
}

pub fn assert_exit(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "unexpected exit code\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

pub fn report(output: &Output) -> JsonReport {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "cannot parse JSON report: {err}\nstdout:\n{}\nstderr:\n{}",
            stdout(output),
            stderr(output)
        )
    })
}

pub fn assert_report(output: &Output, expected: &[(&str, &str)]) {
    let actual = report(output)
        .files
        .into_iter()
        .map(|file| (file.path, file.outcome))
        .collect::<Vec<_>>();
    let expected = expected
        .iter()
        .map(|(path, outcome)| (PathBuf::from(path), (*outcome).to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn copy_tree(source: &Path, destination: &Path) {
    for entry in fs::read_dir(source).expect("read test case") {
        let entry = entry.expect("read test case entry");
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("read test case file type")
            .is_dir()
        {
            fs::create_dir(&target).expect("create test case directory");
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy test case file");
        }
    }
}
