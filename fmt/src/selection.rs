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

use std::path::Path;
use std::path::PathBuf;

use exn::ensure;
use exn::Result;
use exn::ResultExt;
use ignore::overrides::OverrideBuilder;
use walkdir::WalkDir;

use crate::error::Error;
use crate::git::GitContext;

pub struct Selection {
    basedir: PathBuf,
    includes: Vec<String>,
    excludes: Vec<String>,
    git_context: GitContext,
}

impl Selection {
    pub fn new(
        basedir: PathBuf,
        header_path: Option<&String>,
        includes: &[String],
        excludes: &[String],
        use_default_excludes: bool,
        git_context: GitContext,
    ) -> Selection {
        let includes = if includes.is_empty() {
            INCLUDES.iter().map(|s| s.to_string()).collect()
        } else {
            includes.to_vec()
        };

        let input_excludes = excludes;
        let mut excludes = vec![];
        if let Some(path) = header_path.cloned() {
            excludes.push(path);
        }
        if use_default_excludes {
            excludes.extend(EXCLUDES.iter().map(ToString::to_string));
        }
        excludes.extend(input_excludes.to_vec());

        Selection {
            basedir,
            includes,
            excludes,
            git_context,
        }
    }

    pub fn select(self) -> Result<Vec<PathBuf>, Error> {
        log::debug!(
            "selecting files with baseDir: {}, included: {:?}, excluded: {:?}",
            self.basedir.display(),
            self.includes,
            self.excludes,
        );

        let (excludes, reverse_excludes) = {
            let mut excludes = self.excludes;
            let reverse_excludes = excludes
                .extract_if(.., |pat| {
                    if pat.starts_with('!') {
                        pat.remove(0);
                        true
                    } else {
                        false
                    }
                })
                .collect::<Vec<_>>();
            (excludes, reverse_excludes)
        };

        let includes = self.includes;
        ensure!(
            includes.iter().all(|pat| !pat.starts_with('!')),
            Error::new(format!(
                "select files failed; reverse pattern is not allowed for includes: {includes:?}"
            ))
        );

        let ignore = self.git_context.config.ignore.is_auto();
        let result = match self.git_context.repo {
            None => select_files_with_ignore(
                &self.basedir,
                &includes,
                &excludes,
                &reverse_excludes,
                ignore,
            )?,
            Some(repo) => {
                select_files_with_git(&self.basedir, &includes, &excludes, &reverse_excludes, repo)?
            }
        };

        log::debug!("selected files: {:?} (count: {})", result, result.len());
        Ok(result)
    }
}

fn select_files_with_ignore(
    basedir: &PathBuf,
    includes: &[String],
    excludes: &[String],
    reverse_excludes: &[String],
    turn_on_git_ignore: bool,
) -> Result<Vec<PathBuf>, Error> {
    let make_error = || Error::new("failed to select files with ignore crate");

    log::debug!(turn_on_git_ignore; "Selecting files with ignore crate");
    let mut result = vec![];

    let walker = ignore::WalkBuilder::new(basedir)
        .ignore(false) // do not use .ignore file
        .hidden(false) // check hidden files
        .follow_links(true) // proper path name
        .parents(turn_on_git_ignore)
        .git_exclude(turn_on_git_ignore)
        .git_global(turn_on_git_ignore)
        .git_ignore(turn_on_git_ignore)
        .overrides({
            let mut builder = OverrideBuilder::new(basedir);
            for pat in includes.iter() {
                builder.add(pat).or_raise(make_error)?;
            }
            for pat in excludes.iter() {
                let pat = format!("!{pat}");
                builder.add(pat.as_str()).or_raise(make_error)?;
            }
            for pat in reverse_excludes.iter() {
                builder.add(pat).or_raise(make_error)?;
            }
            builder.build().or_raise(make_error)?
        })
        .build();

    for mat in walker {
        let mat = mat.or_raise(make_error)?;
        if mat.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            result.push(mat.into_path())
        }
    }

    Ok(result)
}

fn select_files_with_git(
    basedir: &Path,
    includes: &[String],
    excludes: &[String],
    reverse_excludes: &[String],
    repo: gix::Repository,
) -> Result<Vec<PathBuf>, Error> {
    log::debug!("selecting files with git helper");
    let mut result = vec![];

    let matcher = {
        let make_error = || Error::new("failed to select files with ignore crate");

        let mut builder = OverrideBuilder::new(basedir);
        for pat in includes.iter() {
            builder.add(pat).or_raise(make_error)?;
        }
        for pat in excludes.iter() {
            let pat = format!("!{pat}");
            builder.add(pat.as_str()).or_raise(make_error)?;
        }
        for pat in reverse_excludes.iter() {
            builder.add(pat).or_raise(make_error)?;
        }
        builder.build().or_raise(make_error)?
    };

    let basedir = basedir.canonicalize().or_raise(|| {
        Error::new(format!(
            "cannot resolve absolute path: {}",
            basedir.display()
        ))
    })?;
    let mut it = WalkDir::new(basedir.clone())
        .follow_links(false)
        .into_iter();

    let workdir = repo.workdir().expect("workdir cannot be absent");
    let workdir = workdir.canonicalize().or_raise(|| {
        Error::new(format!(
            "cannot resolve absolute path: {}",
            workdir.display()
        ))
    })?;
    let worktree = repo.worktree().expect("worktree cannot be absent");
    let mut excludes = worktree
        .excludes(None)
        .or_raise(|| Error::new("cannot create gix exclude stack"))?;
    let index = repo
        .index_or_empty()
        .or_raise(|| Error::new("cannot open gix index"))?;

    while let Some(entry) = it.next() {
        let entry = entry.or_raise(|| Error::new("cannot traverse directory"))?;
        let path = entry.path();
        let file_type = entry.file_type();
        if !file_type.is_file() && !file_type.is_dir() {
            log::debug!(file_type:?; "skip file: {path:?}");
            continue;
        }

        let rela_path = path
            .strip_prefix(&workdir)
            .expect("git repository encloses iteration");
        let mode = Some(if file_type.is_dir() {
            gix::index::entry::Mode::DIR
        } else {
            gix::index::entry::Mode::FILE
        });
        let platform = excludes
            .at_path(rela_path, mode)
            .or_raise(|| Error::new("cannot check gix exclude"))?;

        if file_type.is_dir() {
            if platform.is_excluded() {
                let rela = gix::path::try_into_bstr(rela_path)
                    .or_raise(|| Error::new("cannot convert path to git path"))?;

                if !index.path_is_directory(rela.as_ref()) {
                    log::debug!(path:?, rela_path:?; "skip git ignored directory");
                    it.skip_current_dir();
                    continue;
                }
            }
            if matcher.matched(rela_path, file_type.is_dir()).is_ignore() {
                log::debug!(path:?, rela_path:?; "skip glob ignored directory");
                it.skip_current_dir();
                continue;
            }
        } else if file_type.is_file() {
            if platform.is_excluded() {
                let rela = gix::path::try_into_bstr(rela_path)
                    .or_raise(|| Error::new("cannot convert path to git path"))?;

                if index.entry_by_path(rela.as_ref()).is_none() {
                    log::debug!(path:?, rela_path:?; "skip git ignored file");
                    continue;
                }
            }
            if !matcher
                .matched(rela_path, file_type.is_dir())
                .is_whitelist()
            {
                log::debug!(path:?, rela_path:?; "skip glob ignored file");
                continue;
            }
            result.push(path.to_path_buf());
        }
    }

    Ok(result)
}

pub const INCLUDES: [&str; 1] = ["**"];
pub const EXCLUDES: [&str; 141] = [
    // Miscellaneous typical temporary files
    "**/*~",
    "**/#*#",
    "**/.#*",
    "**/%*%",
    "**/._*",
    "**/.repository/**",
    "**/*.lck",
    // HawkEye's own atomic-write temp files, leaked on SIGKILL/power loss; skip on re-walk.
    "**/.*.hawkeye-*.tmp",
    // CVS
    "**/CVS",
    "**/CVS/**",
    "**/.cvsignore",
    // RCS
    "**/RCS",
    "**/RCS/**",
    // SCCS
    "**/SCCS",
    "**/SCCS/**",
    // Visual SourceSafe
    "**/vssver.scc",
    // Subversion
    "**/.svn",
    "**/.svn/**",
    // Arch
    "**/.arch-ids",
    "**/.arch-ids/**",
    // Bazaar
    "**/.bzr",
    "**/.bzr/**",
    // SurroundSCM
    "**/.MySCMServerInfo",
    // Mac
    "**/.DS_Store",
    // Docker
    ".dockerignore",
    // Serena Dimensions Version 10
    "**/.metadata",
    "**/.metadata/**",
    // Mercurial
    "**/.hg",
    "**/.hg/**",
    "**/.hgignore",
    // git
    "**/.git",
    "**/.git/**",
    "**/.gitattributes",
    "**/.gitignore",
    "**/.gitkeep",
    "**/.gitmodules",
    // BitKeeper
    "**/BitKeeper",
    "**/BitKeeper/**",
    "**/ChangeSet",
    "**/ChangeSet/**",
    // darcs
    "**/_darcs",
    "**/_darcs/**",
    "**/.darcsrepo",
    "**/.darcsrepo/**",
    "**/-darcs-backup*",
    "**/.darcs-temp-mail",
    // maven project's temporary files
    "**/target/**",
    "**/test-output/**",
    "**/release.properties",
    "**/dependency-reduced-pom.xml",
    "**/release-pom.xml",
    "**/pom.xml.releaseBackup",
    "**/pom.xml.versionsBackup",
    // Node
    "**/node/**",
    "**/node_modules/**",
    // Yarn
    "**/.yarn/**",
    "**/yarn.lock",
    // pnpm
    "pnpm-lock.yaml",
    // Golang
    "**/go.sum",
    // Cargo
    "**/Cargo.lock",
    // code coverage tools
    "**/cobertura.ser",
    "**/.clover/**",
    "**/jacoco.exec",
    // eclipse project files
    "**/.classpath",
    "**/.project",
    "**/.settings/**",
    // IDEA project files
    "**/*.iml",
    "**/*.ipr",
    "**/*.iws",
    "**/.idea/**",
    // Netbeans
    "**/nb-configuration.xml",
    // Hibernate Validator Annotation Processor
    "**/.factorypath",
    // descriptors
    "**/MANIFEST.MF",
    // License files
    "**/LICENSE",
    "**/LICENSE_HEADER",
    // binary files - images
    "**/*.jpg",
    "**/*.png",
    "**/*.gif",
    "**/*.ico",
    "**/*.bmp",
    "**/*.tiff",
    "**/*.tif",
    "**/*.cr2",
    "**/*.xcf",
    // binary files - programs
    "**/*.class",
    "**/*.exe",
    "**/*.dll",
    "**/*.so",
    // checksum files
    "**/*.md5",
    "**/*.sha1",
    "**/*.sha256",
    "**/*.sha512",
    // Security files
    "**/*.asc",
    "**/*.jks",
    "**/*.keytab",
    "**/*.lic",
    "**/*.p12",
    "**/*.pub",
    // binary files - archives
    "**/*.jar",
    "**/*.zip",
    "**/*.rar",
    "**/*.tar",
    "**/*.tar.gz",
    "**/*.tar.bz2",
    "**/*.gz",
    "**/*.7z",
    // ServiceLoader files
    "**/META-INF/services/**",
    // Markdown files
    "**/*.md",
    // Office documents
    "**/*.xls",
    "**/*.doc",
    "**/*.odt",
    "**/*.ods",
    "**/*.pdf",
    // Travis
    "**/.travis.yml",
    // AppVeyor
    "**/.appveyor.yml",
    "**/appveyor.yml",
    // CircleCI
    "**/.circleci",
    "**/.circleci/**",
    // SourceHut
    "**/.build.yml",
    // Maven 3.3+ configs
    "**/jvm.config",
    "**/maven.config",
    // Wrappers
    "**/gradlew",
    "**/gradlew.bat",
    "**/gradle-wrapper.properties",
    "**/mvnw",
    "**/mvnw.cmd",
    "**/maven-wrapper.properties",
    "**/MavenWrapperDownloader.java",
    // flash
    "**/*.swf",
    // json files
    "**/*.json",
    // fonts
    "**/*.svg",
    "**/*.eot",
    "**/*.otf",
    "**/*.ttf",
    "**/*.woff",
    "**/*.woff2",
    // logs
    "**/*.log",
    // office documents
    "**/*.xlsx",
    "**/*.docx",
    "**/*.ppt",
    "**/*.pptx",
];
