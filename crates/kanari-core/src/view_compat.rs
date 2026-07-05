// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::BlockchainEngine;

impl BlockchainEngine {
    pub fn execute_view_function(
        &self,
        _package: &str,
        _module: &str,
        _function: &str,
        _type_args: &[String],
        _args: &[Vec<u8>],
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        Err(anyhow::anyhow!(
            "Legacy viewFunction is disabled on the object-centric protocol path"
        ))
    }
}
