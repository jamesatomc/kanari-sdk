// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result as AnyhowResult;
use std::fmt::Display;
use thiserror::Error;

/// Shared error type for small invariant / unwrap-style helpers used across Kanari crates.
#[derive(Error, Debug)]
pub enum KanariError {
    #[error("Missing required value: {0}")]
    MissingValue(&'static str),

    #[error("Operation failed: {context}: {details}")]
    OperationFailed {
        context: &'static str,
        details: String,
    },

    #[error("Kanari invariant violated: {0}")]
    InvariantViolation(&'static str),
}

pub type Result<T> = std::result::Result<T, KanariError>;

/// Shared helpers for places where runtime code needs a short, consistent
/// replacement for ad-hoc `unwrap()` / `expect()` calls.
pub trait KanariUnwrapExt<T> {
    /// Convert a missing/failing value into an anyhow error with stable context.
    fn require(self, message: &'static str) -> AnyhowResult<T>;

    /// Convert a missing/failing value into a typed Kanari error.
    fn typed(self, message: &'static str) -> Result<T>;

    /// Panic with a consistent invariant message for places that are intentionally infallible.
    fn invariant(self, message: &'static str) -> T;
}

impl<T> KanariUnwrapExt<T> for Option<T> {
    fn require(self, message: &'static str) -> AnyhowResult<T> {
        self.typed(message).map_err(Into::into)
    }

    fn typed(self, message: &'static str) -> Result<T> {
        self.ok_or(KanariError::MissingValue(message))
    }

    fn invariant(self, message: &'static str) -> T {
        match self {
            Some(value) => value,
            None => panic!("{}", KanariError::InvariantViolation(message)),
        }
    }
}

impl<T, E> KanariUnwrapExt<T> for std::result::Result<T, E>
where
    E: Display,
{
    fn require(self, message: &'static str) -> AnyhowResult<T> {
        self.typed(message).map_err(Into::into)
    }

    fn typed(self, message: &'static str) -> Result<T> {
        self.map_err(|error| KanariError::OperationFailed {
            context: message,
            details: error.to_string(),
        })
    }

    fn invariant(self, message: &'static str) -> T {
        match self {
            Ok(value) => value,
            Err(error) => panic!(
                "{}",
                KanariError::OperationFailed {
                    context: message,
                    details: error.to_string(),
                }
            ),
        }
    }
}
