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
use crate::Result;
use crate::Status;

/// A replacement over a byte range of the analyzed input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    range: Range<usize>,
    expected: String,
    replacement: String,
}

impl Edit {
    /// Creates an edit and captures the original range to reject stale application.
    pub fn new(input: &str, range: Range<usize>, replacement: impl Into<String>) -> Result<Self> {
        validate_range(input, &range)?;
        Ok(Self {
            expected: input[range.clone()].to_owned(),
            range,
            replacement: replacement.into(),
        })
    }

    /// Returns the byte range replaced by this edit.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// Returns the replacement text.
    pub fn replacement(&self) -> &str {
        &self.replacement
    }

    /// Applies the edit only when the analyzed range still contains the expected bytes.
    pub fn apply(&self, input: &str) -> Result<String> {
        validate_range(input, &self.range)?;
        if input[self.range.clone()] != self.expected {
            return Err(Error::StaleEdit);
        }

        let mut output =
            String::with_capacity(input.len() - self.range.len() + self.replacement.len());
        output.push_str(&input[..self.range.start]);
        output.push_str(&self.replacement);
        output.push_str(&input[self.range.end..]);
        Ok(output)
    }
}

/// The status and optional safe mutation produced for one operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditPlan {
    status: Status,
    edit: Option<Edit>,
}

impl EditPlan {
    pub(crate) fn new(status: Status, edit: Option<Edit>) -> Self {
        Self { status, edit }
    }

    /// Returns the analysis status.
    pub fn status(&self) -> Status {
        self.status
    }

    /// Returns the safe edit, if this operation requires one.
    pub fn edit(&self) -> Option<&Edit> {
        self.edit.as_ref()
    }

    /// Applies the planned edit, or returns the input unchanged when no edit is needed.
    pub fn apply(&self, input: &str) -> Result<String> {
        match &self.edit {
            Some(edit) => edit.apply(input),
            None => Ok(input.to_owned()),
        }
    }

    pub(crate) fn into_parts(self) -> (Status, Option<Edit>) {
        (self.status, self.edit)
    }
}

fn validate_range(input: &str, range: &Range<usize>) -> Result<()> {
    if range.start > range.end
        || range.end > input.len()
        || !input.is_char_boundary(range.start)
        || !input.is_char_boundary(range.end)
    {
        return Err(Error::InvalidEdit {
            range: range.clone(),
            input_len: input.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_application_to_changed_input() {
        let edit = Edit::new("abc", 1..2, "x").unwrap();
        assert!(matches!(edit.apply("adc"), Err(Error::StaleEdit)));
    }
}
