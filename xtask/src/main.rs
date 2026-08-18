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

use std::process::Command as StdCommand;

use clap::Parser;
use clap::Subcommand;

#[derive(Parser)]
#[clap(about = "Run repository tasks.")]
struct Command {
    #[clap(subcommand)]
    sub: SubCommand,
}

#[derive(Subcommand)]
enum SubCommand {
    #[clap(about = "Compile all workspace targets.")]
    Build(CommandBuild),
    #[clap(about = "Run workspace quality checks.")]
    Lint(CommandLint),
    #[clap(about = "Run workspace tests.")]
    Test(CommandTest),
}

#[derive(Parser)]
struct CommandBuild {
    #[arg(long, help = "Assert that `Cargo.lock` will remain unchanged.")]
    locked: bool,
}

#[derive(Parser)]
struct CommandTest {
    #[arg(long, help = "Run tests serially and do not capture output.")]
    no_capture: bool,
}

#[derive(Parser)]
#[clap(name = "lint")]
struct CommandLint {
    #[arg(long, help = "Automatically apply available lint and format fixes.")]
    fix: bool,
}

fn find_command(cmd: &str) -> StdCommand {
    match which::which(cmd) {
        Ok(exe) => {
            let mut cmd = StdCommand::new(exe);
            cmd.current_dir(env!("CARGO_WORKSPACE_DIR"));
            cmd
        }
        Err(err) => {
            panic!("{cmd} not found: {err}");
        }
    }
}

fn ensure_installed(bin: &str, crate_name: &str) {
    if which::which(bin).is_err() {
        let mut cmd = find_command("cargo");
        cmd.args(["install", crate_name]);
        run_command(cmd);
    }
}

fn run_command(mut cmd: StdCommand) {
    println!("{cmd:?}");
    let status = cmd.status().unwrap();
    assert!(status.success(), "command failed: {status}");
}

fn make_build_cmd(locked: bool) -> StdCommand {
    let mut cmd = find_command("cargo");
    cmd.args([
        "build",
        "--workspace",
        "--all-features",
        "--tests",
        "--examples",
        "--benches",
        "--bins",
    ]);
    if locked {
        cmd.arg("--locked");
    }
    cmd
}

fn make_test_cmd(no_capture: bool) -> StdCommand {
    let mut cmd = find_command("cargo");
    cmd.args(["test", "--workspace", "--all-features"]);
    if no_capture {
        cmd.args(["--", "--nocapture"]);
    }
    cmd
}

fn make_library_check_cmd() -> StdCommand {
    let mut cmd = find_command("cargo");
    cmd.args([
        "check",
        "--package",
        "hawkeye",
        "--lib",
        "--no-default-features",
    ]);
    cmd
}

fn make_format_cmd(fix: bool) -> StdCommand {
    let mut cmd = find_command("cargo");
    cmd.args(["+nightly", "fmt", "--all"]);
    if !fix {
        cmd.arg("--check");
    }
    cmd
}

fn make_clippy_cmd(fix: bool) -> StdCommand {
    let mut cmd = find_command("cargo");
    cmd.args([
        "+nightly",
        "clippy",
        "--tests",
        "--all-features",
        "--all-targets",
        "--workspace",
    ]);
    if fix {
        cmd.args(["--allow-staged", "--allow-dirty", "--fix"]);
    } else {
        cmd.args(["--", "-D", "warnings"]);
    }
    cmd
}

fn make_doc_cmd() -> StdCommand {
    let mut cmd = find_command("cargo");
    cmd.env("RUSTDOCFLAGS", "-D warnings --cfg docsrs");
    cmd.args([
        "+nightly",
        "doc",
        "--workspace",
        "--all-features",
        "--no-deps",
    ]);
    cmd
}

fn make_hawkeye_cmd(fix: bool) -> StdCommand {
    let mut cmd = find_command("cargo");
    cmd.args([
        "run",
        "--quiet",
        "--package",
        "hawkeye",
        "--bin",
        "hawkeye",
        "--",
    ]);
    if fix {
        cmd.args(["format"]);
    } else {
        cmd.args(["check"]);
    }
    cmd
}

fn make_typos_cmd() -> StdCommand {
    ensure_installed("typos", "typos-cli");
    find_command("typos")
}

fn make_taplo_cmd(fix: bool) -> StdCommand {
    ensure_installed("taplo", "taplo-cli");
    let mut cmd = find_command("taplo");
    if fix {
        cmd.args(["format"]);
    } else {
        cmd.args(["format", "--check"]);
    }
    cmd
}

fn main() {
    match Command::parse().sub {
        SubCommand::Build(cmd) => run_command(make_build_cmd(cmd.locked)),
        SubCommand::Test(cmd) => {
            run_command(make_library_check_cmd());
            run_command(make_test_cmd(cmd.no_capture));
        }
        SubCommand::Lint(cmd) => {
            run_command(make_clippy_cmd(cmd.fix));
            run_command(make_format_cmd(cmd.fix));
            run_command(make_taplo_cmd(cmd.fix));
            run_command(make_typos_cmd());
            run_command(make_hawkeye_cmd(cmd.fix));
            run_command(make_doc_cmd());
        }
    }
}
