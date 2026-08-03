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
use ignore::overrides::Override;
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

    /// Select all the files under the base directory.
    pub fn select(self) -> Result<Vec<PathBuf>, Error> {
        self.do_select(None)
    }

    /// Select only the given files and directories, which are resolved against the base directory.
    ///
    /// This is meant for large repositories, where walking the whole base directory (and its Git
    /// history) is way more expensive than processing the files one is interested in.
    ///
    /// Directories are walked as [`Selection::select`] does. Files are selected as long as they
    /// match `includes` and `excludes`; listing a file explicitly overrides the Git ignore rules,
    /// like `git add --force` does. Paths that do not exist, or that are outside the base
    /// directory, are skipped with a warning.
    pub fn select_paths(self, paths: &[PathBuf]) -> Result<Vec<PathBuf>, Error> {
        self.do_select(Some(paths))
    }

    fn do_select(self, paths: Option<&[PathBuf]>) -> Result<Vec<PathBuf>, Error> {
        log::debug!(
            "selecting files with baseDir: {}, included: {:?}, excluded: {:?}, paths: {:?}",
            self.basedir.display(),
            self.includes,
            self.excludes,
            paths,
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

        let matcher = build_matcher(&self.basedir, &includes, &excludes, &reverse_excludes)?;
        let (dirs, files) = match paths {
            None => (vec![self.basedir.clone()], vec![]),
            Some(paths) => resolve_paths(&self.basedir, paths)?,
        };

        let ignore = self.git_context.config.ignore.is_auto();
        let mut result = vec![];
        match self.git_context.repo {
            None => {
                for dir in &dirs {
                    result.extend(select_files_with_ignore(dir, &matcher, ignore)?);
                }
                for file in &files {
                    // the ignore crate matches paths relative to the base directory
                    if is_selected_file(&matcher, &file.rela_path) {
                        result.push(file.path.clone());
                    }
                }
            }
            Some(repo) => {
                for dir in &dirs {
                    result.extend(select_files_with_git(dir, &matcher, &repo)?);
                }
                if !files.is_empty() {
                    let workdir = repo.workdir().expect("workdir cannot be absent");
                    let workdir = canonicalize(workdir)?;
                    for file in &files {
                        // the git helper matches paths relative to the workdir
                        let rela_path = match file.path.strip_prefix(&workdir) {
                            Ok(rela_path) => rela_path,
                            Err(_) => {
                                log::warn!(
                                    "skip file outside of the git repository: {}",
                                    file.path.display()
                                );
                                continue;
                            }
                        };
                        if is_selected_file(&matcher, rela_path) {
                            result.push(file.path.clone());
                        }
                    }
                }
            }
        }

        if paths.is_some() {
            // a file can be selected both on its own and as part of a selected directory
            result.sort();
            result.dedup();
        }

        log::debug!("selected files: {:?} (count: {})", result, result.len());
        Ok(result)
    }
}

/// A file that has been explicitly passed for processing.
struct FileToSelect {
    /// Path relative to the base directory, that is, what includes and excludes are anchored to.
    rela_path: PathBuf,
    /// Absolute path, as the directory walkers report the files they select.
    path: PathBuf,
}

/// Resolve the given paths against the base directory, and split them into directories to walk and
/// files to select. Paths that cannot be resolved, or that escape the base directory, are dropped.
fn resolve_paths(
    basedir: &Path,
    paths: &[PathBuf],
) -> Result<(Vec<PathBuf>, Vec<FileToSelect>), Error> {
    let absolute_basedir = canonicalize(basedir)?;

    let mut dirs = vec![];
    let mut files = vec![];
    for path in paths {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            basedir.join(path)
        };

        let absolute_path = match path.canonicalize() {
            Ok(absolute_path) => absolute_path,
            Err(err) => {
                log::warn!(err:?; "skip path that cannot be resolved: {}", path.display());
                continue;
            }
        };

        let rela_path = match absolute_path.strip_prefix(&absolute_basedir) {
            Ok(rela_path) => rela_path.to_path_buf(),
            Err(_) => {
                log::warn!(
                    "skip path outside of baseDir {}: {}",
                    basedir.display(),
                    path.display()
                );
                continue;
            }
        };

        if absolute_path.is_dir() {
            // walk directories with the absolute path, as the git helper does
            dirs.push(absolute_path);
        } else if absolute_path.is_file() {
            files.push(FileToSelect {
                rela_path,
                path: absolute_path,
            });
        } else {
            log::warn!(
                "skip path that is neither a file nor a directory: {}",
                path.display()
            );
        }
    }

    Ok((dirs, files))
}

/// Whether an explicitly passed file matches the configured includes and excludes.
fn is_selected_file(matcher: &Override, rela_path: &Path) -> bool {
    for dir in rela_path.ancestors().skip(1) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if matcher.matched(dir, true).is_ignore() {
            log::debug!(rela_path:?, dir:?; "skip glob ignored file under ignored directory");
            return false;
        }
    }

    if !matcher.matched(rela_path, false).is_whitelist() {
        log::debug!(rela_path:?; "skip glob ignored file");
        return false;
    }

    true
}

fn build_matcher(
    basedir: &Path,
    includes: &[String],
    excludes: &[String],
    reverse_excludes: &[String],
) -> Result<Override, Error> {
    let make_error = || Error::new("failed to build the include and exclude matcher");

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
    builder.build().or_raise(make_error)
}

fn canonicalize(path: &Path) -> Result<PathBuf, Error> {
    path.canonicalize()
        .or_raise(|| Error::new(format!("cannot resolve absolute path: {}", path.display())))
}

fn select_files_with_ignore(
    root: &Path,
    matcher: &Override,
    turn_on_git_ignore: bool,
) -> Result<Vec<PathBuf>, Error> {
    let make_error = || Error::new("failed to select files with ignore crate");

    log::debug!(turn_on_git_ignore; "Selecting files with ignore crate");
    let mut result = vec![];

    let walker = ignore::WalkBuilder::new(root)
        .ignore(false) // do not use .ignore file
        .hidden(false) // check hidden files
        .follow_links(true) // proper path name
        .parents(turn_on_git_ignore)
        .git_exclude(turn_on_git_ignore)
        .git_global(turn_on_git_ignore)
        .git_ignore(turn_on_git_ignore)
        .overrides(matcher.clone())
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
    root: &Path,
    matcher: &Override,
    repo: &gix::Repository,
) -> Result<Vec<PathBuf>, Error> {
    log::debug!("selecting files with git helper");
    let mut result = vec![];

    let root = canonicalize(root)?;
    let mut it = WalkDir::new(root).follow_links(false).into_iter();

    let workdir = repo.workdir().expect("workdir cannot be absent");
    let workdir = canonicalize(workdir)?;
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
pub const EXCLUDES: [&str; 140] = [
    // Miscellaneous typical temporary files
    "**/*~",
    "**/#*#",
    "**/.#*",
    "**/%*%",
    "**/._*",
    "**/.repository/**",
    "**/*.lck",
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
