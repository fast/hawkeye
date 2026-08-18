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

use std::ops::Range;

/// Iterates line contents without terminators and their full byte ranges in the input.
pub fn iter(input: &str, start: usize) -> impl Iterator<Item = (&str, Range<usize>)> {
    let mut position = start;
    input[start..].split_inclusive('\n').map(move |line| {
        let raw_range = position..position + line.len();
        position = raw_range.end;
        let content = if let Some(content) = line.strip_suffix('\n') {
            content.strip_suffix('\r').unwrap_or(content)
        } else {
            line
        };
        (content, raw_range)
    })
}
