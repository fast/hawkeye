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

use crate::Error;
use crate::ErrorKind;

/// A single, proven replacement in an original UTF-8 source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    range: Range<usize>,
    replacement: String,
}

impl Edit {
    pub(crate) fn new(range: Range<usize>, replacement: String) -> Self {
        Self { range, replacement }
    }

    /// Applies the edit after checking its UTF-8 byte boundaries.
    pub fn apply(&self, input: &str) -> Result<String, Error> {
        if self.range.start > self.range.end
            || self.range.end > input.len()
            || !input.is_char_boundary(self.range.start)
            || !input.is_char_boundary(self.range.end)
        {
            return Err(Error::new(
                ErrorKind::Unexpected,
                format!(
                    "invalid edit range {:?} for an input of {} bytes",
                    self.range,
                    input.len()
                ),
            ));
        }
        let mut output = String::with_capacity(
            input.len() - (self.range.end - self.range.start) + self.replacement.len(),
        );
        output.push_str(&input[..self.range.start]);
        output.push_str(&self.replacement);
        output.push_str(&input[self.range.end..]);
        Ok(output)
    }
}
