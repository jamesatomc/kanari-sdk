// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use serde_json;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetworkConfig {
    pub node_address: String,
    pub port: u16,
    pub peers: Vec<String>,
    pub chain_id: String,
    pub max_connections: u32,
    pub api_enabled: bool,
    pub localhost_only: bool, // New field to restrict to localhost only
    pub use_tls: bool,        // Whether to use TLS encryption
    pub trusted_peers: Vec<String>, // List of trusted peer IDs or addresses
}

impl NetworkConfig {
    // Loads configuration from a specified file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let config: NetworkConfig = serde_json::from_str(&contents)?;
        Ok(config)
    }

    // Saves the current configuration to a specified file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let serialized = serde_json::to_string_pretty(&self)?;
        let mut file = File::create(path)?;
        file.write_all(serialized.as_bytes())?;
        Ok(())
    }

    // Updates the configuration with new values
    pub fn update(&mut self, new_config: NetworkConfig) {
        self.node_address = new_config.node_address;
        self.port = new_config.port;
        self.peers = new_config.peers;
        self.chain_id = new_config.chain_id;
        self.max_connections = new_config.max_connections;
        self.api_enabled = new_config.api_enabled;
        self.localhost_only = new_config.localhost_only;
        self.use_tls = new_config.use_tls;
        self.trusted_peers = new_config.trusted_peers;
    }

    // New method to add trusted peers
    pub fn add_trusted_peer(&mut self, peer: String) {
        if !self.trusted_peers.contains(&peer) {
            self.trusted_peers.push(peer);
        }
    }

    // New method to check if a peer is trusted
    pub fn is_trusted_peer(&self, peer: &str) -> bool {
        self.trusted_peers.contains(&peer.to_string())
    }
}

// Default implementation
impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            node_address: "0.0.0.0".to_string(),
            port: 30030,
            peers: Vec::new(),
            chain_id: "kanari-local".to_string(),
            max_connections: 100,
            api_enabled: true,
            localhost_only: false,
            use_tls: false,
            trusted_peers: Vec::new(),
        }
    }
}
