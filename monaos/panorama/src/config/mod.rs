// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0
//
use network::NetworkConfig;
use std::io;

// Re-export all functions from common for backward compatibility
pub use common::{
    configure_network_settings, ensure_network_config, get_network_config, init_default_config,
    load_config, load_kanari_config, prompt_for_value, save_config, save_kanari_config,
    setup_network_config, update_network_config,
};

/// Function to configure the network settings (backward compatibility wrapper)
pub fn configure_network(chain_id: &str) -> io::Result<NetworkConfig> {
    setup_network_config(chain_id)
}
