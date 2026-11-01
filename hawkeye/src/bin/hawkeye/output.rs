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

use std::io::Write;

use exn::Result;
use exn::ResultExt;
use hawkeye::FileOutcome;
use hawkeye::Report;

use super::Error;
use super::OutputFormat;

impl OutputFormat {
    pub(super) fn render(self, report: &Report) -> Result<Vec<u8>, Error> {
        let mut output = Vec::new();
        match self {
            Self::Json => {
                output = serde_json::to_vec_pretty(report)
                    .or_raise(|| Error::new("cannot serialize JSON report"))?;
                output.push(b'\n');
            }
            Self::Human => {
                let mut changes = 0;
                let mut conflicts = 0;
                let mut unsupported = 0;
                for file in &report.files {
                    let label = match file.outcome {
                        FileOutcome::Clean => continue,
                        FileOutcome::Add => {
                            changes += 1;
                            "add"
                        }
                        FileOutcome::Replace => {
                            changes += 1;
                            "replace"
                        }
                        FileOutcome::Remove => {
                            changes += 1;
                            "remove"
                        }
                        FileOutcome::Conflict => {
                            conflicts += 1;
                            "conflict"
                        }
                        FileOutcome::Unsupported => {
                            unsupported += 1;
                            "unsupported"
                        }
                    };
                    writeln!(output, "{label:>11}  {}", file.path.display())
                        .expect("writing to a Vec cannot fail");
                }
                let files = report.files.len();
                let file_label = if files == 1 { "file" } else { "files" };
                let change_label = if changes == 1 { "change" } else { "changes" };
                let conflict_label = if conflicts == 1 {
                    "conflict"
                } else {
                    "conflicts"
                };
                writeln!(
                    output,
                    "{files} {file_label}, {changes} {change_label}, {conflicts} {conflict_label}, {unsupported} unsupported"
                )
                .expect("writing to a Vec cannot fail");
            }
        }
        Ok(output)
    }
}
