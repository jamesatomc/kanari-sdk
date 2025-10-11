// Copyright (c) RoochNetwork
// SPDX-License-Identifier: Apache-2.0

use crate::STATIC_FRAMEWORK_DIR;
use framework_types::addresses::*;
use move_core_types::{account_address::AccountAddress, errmap::ErrorMapping};
use once_cell::sync::Lazy;
use std::collections::BTreeMap;

pub static ERROR_DESCRIPTIONS: Lazy<BTreeMap<AccountAddress, ErrorMapping>> = Lazy::new(|| {
    let mut error_descriptions = BTreeMap::new();

    let kanari_library_err: ErrorMapping = bcs::from_bytes(
        STATIC_FRAMEWORK_DIR
            .get_file("latest/kanari_library_error_description.errmap")
            .unwrap()
            .contents(),
    )
    .unwrap();

    error_descriptions.insert(KANARI_LIBRARY_ADDRESS, kanari_library_err);

    error_descriptions
});

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_error_descriptions() {
        let error_descriptions = ERROR_DESCRIPTIONS.clone();
        let error_mapping = error_descriptions.get(&KANARI_LIBRARY_ADDRESS).unwrap();
        //println!("{:?}",error_mapping.module_error_maps);
        let description = error_mapping.get_explanation("0x6::block", 1);
        //println!("{:?}",description);
        assert!(description.is_some());
        let description = description.unwrap();
        assert_eq!(description.code_name.as_str(), "ErrorAlreadyExists");
    }
}
