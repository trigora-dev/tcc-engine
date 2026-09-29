// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    UnsupportedValue,
    InvalidEncoding(String),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::UnsupportedValue => write!(f, "unsupported value encoding"),
            StateError::InvalidEncoding(message) => {
                write!(f, "continuation encoding is invalid: {message}")
            }
        }
    }
}

impl std::error::Error for StateError {}
