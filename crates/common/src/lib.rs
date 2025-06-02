// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use dirs;
use network::NetworkConfig;
use serde_yaml::{Mapping, Value};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

// Path utility functions
pub fn get_kari_dir() -> PathBuf {
    let mut path = dirs::home_dir().expect("Unable to find home directory");
    path.push(".kari");
    fs::create_dir_all(&path).expect("Unable to create .kari directory");
    path
}

/// Get path to the kanari.yaml configuration file
pub fn get_kanari_config_path() -> PathBuf {
    let mut config_dir = get_kari_dir();
    config_dir.push("kanari_config");
    fs::create_dir_all(&config_dir).expect("Unable to create kanari_config directory");
    config_dir.push("kanari.yaml");
    config_dir
}

/// Load configuration from kanari.yaml file
pub fn load_kanari_config() -> io::Result<Value> {
    let config_path = get_kanari_config_path();

    // Return empty config if file doesn't exist
    if !config_path.exists() {
        return Ok(Value::Mapping(Mapping::new()));
    }

    // Read and parse config file
    let config_str = fs::read_to_string(&config_path)?;

    // Return empty config if file is empty
    if config_str.trim().is_empty() {
        return Ok(Value::Mapping(Mapping::new()));
    }

    // Parse YAML with error handling
    let config: Value = serde_yaml::from_str(&config_str).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse kanari.yaml file: {}", e),
        )
    })?;

    Ok(config)
}

/// Save configuration to kanari.yaml file
pub fn save_kanari_config(config: &Value) -> io::Result<()> {
    let config_path = get_kanari_config_path();
    let mut file = File::create(config_path)?;

    // Serialize and save with error handling
    let yaml_str = serde_yaml::to_string(config).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to serialize config: {}", e),
        )
    })?;

    file.write_all(yaml_str.as_bytes())?;
    Ok(())
}

/// Load configuration (now completely from kanari.yaml)
pub fn load_config() -> io::Result<Value> {
    let kanari_config = load_kanari_config()?;

    let active_env_str = match kanari_config.get("active_env").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return Ok(Value::Mapping(Mapping::new())), // No active_env, return empty
    };

    let envs = match kanari_config.get("envs").and_then(|v| v.as_sequence()) {
        Some(s) => s,
        None => return Ok(Value::Mapping(Mapping::new())), // No envs sequence, return empty
    };

    if let Some(active_env_config) = envs
        .iter()
        .find(|env| env.get("alias").and_then(|v| v.as_str()) == Some(active_env_str))
    {
        let mut config_map = Mapping::new();

        let chain_id = match active_env_str {
            "local" => "kari-local-001",
            "dev" => "kari-dev-001",
            "test" => "kari-testnet-001",
            "main" => "kari-mainnet-001",
            _ => "kari-testnet-001", // Default or consider error
        };
        config_map.insert(
            Value::String("chain_id".to_string()),
            Value::String(chain_id.to_string()),
        );

        if let Some(addr) = kanari_config.get("active_address").and_then(|v| v.as_str()) {
            config_map.insert(
                Value::String("address".to_string()),
                Value::String(addr.to_string()),
            );
        }

        if let Some(rpc_url) = active_env_config.get("rpc").and_then(|v| v.as_str()) {
            let rpc_port = if rpc_url.starts_with("http://127.0.0.1:")
                || rpc_url.starts_with("http://localhost:")
            {
                rpc_url
                    .split(':')
                    .nth(2)
                    .and_then(|p_str| p_str.parse::<u64>().ok())
                    .unwrap_or(30030)
            } else {
                30030 // Default for remote or unparseable local
            };
            config_map.insert(
                Value::String("rpc_port".to_string()),
                Value::Number(serde_yaml::Number::from(rpc_port)),
            );
        } else {
            config_map.insert(
                Value::String("rpc_port".to_string()),
                Value::Number(serde_yaml::Number::from(30030u64)),
            ); // Default if rpc field is missing
        }

        return Ok(Value::Mapping(config_map));
    }

    Ok(Value::Mapping(Mapping::new())) // Active environment not found in envs list
}

/// Save configuration to kanari.yaml file
pub fn save_config(config_to_save: &Value) -> io::Result<()> {
    let mut kanari_config = load_kanari_config().unwrap_or_else(|_| Value::Mapping(Mapping::new()));

    let active_env_alias = match kanari_config.get("active_env").and_then(|v| v.as_str()) {
        Some(alias) => alias.to_string(),
        None => return Ok(()), // No active_env to update
    };

    let config_to_save_map = match config_to_save.as_mapping() {
        Some(map) => map,
        None => return Ok(()), // Nothing to save if not a mapping
    };

    if let Some(kanari_config_map) = kanari_config.as_mapping_mut() {
        // Update active_address if "address" is in config_to_save
        if let Some(addr_val) = config_to_save_map.get("address").and_then(|v| v.as_str()) {
            kanari_config_map.insert(
                Value::String("active_address".to_string()),
                Value::String(addr_val.to_string()),
            );
        }

        // Update RPC URL in the active environment if "rpc_port" is in config_to_save
        if let Some(envs) = kanari_config_map
            .get_mut("envs")
            .and_then(|v| v.as_sequence_mut())
        {
            if let Some(env_to_update) = envs
                .iter_mut()
                .find(|env| env.get("alias").and_then(|v| v.as_str()) == Some(&active_env_alias))
            {
                if let Some(rpc_port_val) =
                    config_to_save_map.get("rpc_port").and_then(|v| v.as_u64())
                {
                    if let Some(env_map_mut) = env_to_update.as_mapping_mut() {
                        env_map_mut.insert(
                            Value::String("rpc".to_string()),
                            Value::String(format!("http://127.0.0.1:{}", rpc_port_val)),
                        );
                    }
                }
            }
        }

        save_kanari_config(&Value::Mapping(kanari_config_map.clone()))?; // Clone because save_kanari_config takes &Value
    }

    Ok(())
}

/// Get current main wallet address
pub fn get_main_wallet() -> Option<String> {
    load_kanari_config().ok().and_then(|config| {
        config
            .get("active_address")
            .and_then(|v| v.as_str())
            .map(String::from)
    })
}

/// Create or update network configuration in kanari.yaml
pub fn configure_network_settings(
    chain_id: &str,
    localhost_only: bool,
    use_tls: bool,
    rpc_port: Option<u16>,
) -> io::Result<NetworkConfig> {
    let mut kanari_config = load_kanari_config()?;

    if let Some(kanari_mapping) = kanari_config.as_mapping_mut() {
        let active_env = kanari_mapping
            .get("active_env")
            .and_then(|v| v.as_str())
            .unwrap_or("local")
            .to_string();

        let port = rpc_port.unwrap_or(30030);

        // Update RPC URL in the active environment
        if let Some(envs) = kanari_mapping
            .get_mut("envs")
            .and_then(|v| v.as_sequence_mut())
        {
            for env in envs {
                if let Some(alias) = env.get("alias").and_then(|v| v.as_str()) {
                    if alias == active_env {
                        env["rpc"] = Value::String(format!("http://127.0.0.1:{}", port));
                        break;
                    }
                }
            }
        }

        // Add network settings to kanari config
        kanari_mapping.insert(
            Value::String("localhost_only".to_string()),
            Value::Bool(localhost_only),
        );
        kanari_mapping.insert(Value::String("use_tls".to_string()), Value::Bool(use_tls));
        kanari_mapping.insert(
            Value::String("rpc_port".to_string()),
            Value::Number(serde_yaml::Number::from(port as u64)),
        );
        kanari_mapping.insert(
            Value::String("chain_id".to_string()),
            Value::String(chain_id.to_string()),
        );

        save_kanari_config(&kanari_config)?;
    }

    Ok(NetworkConfig {
        node_address: if localhost_only {
            "127.0.0.1".to_string()
        } else {
            "0.0.0.0".to_string()
        },
        port: rpc_port.unwrap_or(30030),
        peers: Vec::new(),
        chain_id: chain_id.to_string(),
        max_connections: 100,
        api_enabled: true,
        localhost_only,
        use_tls,
        trusted_peers: Vec::new(),
    })
}

/// Get network configuration from kanari.yaml
pub fn get_network_config() -> io::Result<Option<NetworkConfig>> {
    let kanari_config = load_kanari_config()?;

    if let Some(mapping) = kanari_config.as_mapping() {
        if mapping.contains_key("rpc_port") {
            let localhost_only = mapping
                .get("localhost_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let use_tls = mapping
                .get("use_tls")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let port = mapping
                .get("rpc_port")
                .and_then(|v| v.as_u64())
                .unwrap_or(30030) as u16;

            // Get chain_id and fix if empty
            let mut chain_id = mapping
                .get("chain_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // If chain_id is empty, determine from active_env
            if chain_id.is_empty() {
                let active_env = mapping
                    .get("active_env")
                    .and_then(|v| v.as_str())
                    .unwrap_or("local");

                chain_id = match active_env {
                    "local" => "kari-local-001",
                    "dev" => "kari-dev-001",
                    "test" => "kari-testnet-001",
                    "main" => "kari-mainnet-001",
                    _ => "kari-local-001",
                }
                .to_string();
            }

            return Ok(Some(NetworkConfig {
                node_address: if localhost_only {
                    "127.0.0.1".to_string()
                } else {
                    "0.0.0.0".to_string()
                },
                port,
                peers: Vec::new(),
                chain_id,
                max_connections: 100,
                api_enabled: true,
                localhost_only,
                use_tls,
                trusted_peers: Vec::new(),
            }));
        }
    }

    Ok(None)
}

/// Update network configuration with new port and peers
pub fn update_network_config(
    port: Option<u16>,
    peers: Vec<String>,
    localhost_only: bool,
    use_tls: bool,
) -> io::Result<()> {
    let mut kanari_config = load_kanari_config()?;

    if let Some(kanari_mapping) = kanari_config.as_mapping_mut() {
        if let Some(p) = port {
            kanari_mapping.insert(
                Value::String("rpc_port".to_string()),
                Value::Number(serde_yaml::Number::from(p as u64)),
            );

            // Update active environment RPC URL
            let active_env = kanari_mapping
                .get("active_env")
                .and_then(|v| v.as_str())
                .unwrap_or("local")
                .to_string();
            if let Some(envs) = kanari_mapping
                .get_mut("envs")
                .and_then(|v| v.as_sequence_mut())
            {
                for env in envs {
                    if let Some(alias) = env.get("alias").and_then(|v| v.as_str()) {
                        if alias == active_env {
                            env["rpc"] = Value::String(format!("http://127.0.0.1:{}", p));
                            break;
                        }
                    }
                }
            }
        }

        kanari_mapping.insert(
            Value::String("localhost_only".to_string()),
            Value::Bool(localhost_only),
        );
        kanari_mapping.insert(Value::String("use_tls".to_string()), Value::Bool(use_tls));

        if !peers.is_empty() {
            let peers_yaml: Vec<Value> = peers.into_iter().map(Value::String).collect();
            kanari_mapping.insert(
                Value::String("peers".to_string()),
                Value::Sequence(peers_yaml),
            );
        }

        save_kanari_config(&kanari_config)?;
    }

    Ok(())
}

/// Initialize default kanari.yaml configuration if it doesn't exist
pub fn init_default_config() -> io::Result<()> {
    let config_path = get_kanari_config_path();

    if config_path.exists() {
        return Ok(()); // Already exists, don't overwrite
    }

    let mut config = Mapping::new();

    // Set default keystore path
    let mut keystore_path = get_kari_dir();
    keystore_path.push("kanari_config");
    keystore_path.push("kanari.keystore");

    config.insert(
        Value::String("keystore_path".to_string()),
        Value::String(keystore_path.to_string_lossy().into_owned()),
    );
    config.insert(Value::String("active_address".to_string()), Value::Null);

    // Create default environments
    let envs = vec![
        create_env_config("local", "http://127.0.0.1:30030", "ws://127.0.0.1:30031"),
        create_env_config(
            "dev",
            "https://dev-seed.kanari.site",
            "wss://dev-seed.kanari.site/websocket",
        ),
        create_env_config(
            "test",
            "https://test-seed.kanari.site",
            "wss://test-seed.kanari.site/websocket",
        ),
        create_env_config(
            "main",
            "https://main-seed.kanari.site",
            "wss://main-seed.kanari.site/websocket",
        ),
    ];

    config.insert(Value::String("envs".to_string()), Value::Sequence(envs));
    config.insert(
        Value::String("active_env".to_string()),
        Value::String("local".to_string()),
    );

    save_kanari_config(&Value::Mapping(config))?;
    Ok(())
}

/// Helper function to create environment configuration
fn create_env_config(alias: &str, rpc: &str, ws: &str) -> Value {
    let mut env_map = Mapping::new();
    env_map.insert(
        Value::String("alias".to_string()),
        Value::String(alias.to_string()),
    );
    env_map.insert(
        Value::String("rpc".to_string()),
        Value::String(rpc.to_string()),
    );
    env_map.insert(
        Value::String("ws".to_string()),
        Value::String(ws.to_string()),
    );
    Value::Mapping(env_map)
}

/// Prompt user for configuration values (moved from panorama::config)
pub fn prompt_for_value(prompt: &str, default: &str) -> String {
    loop {
        print!("{} [{}]: ", prompt, default);
        io::stdout().flush().unwrap();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        let trimmed_input = input.trim();
        if trimmed_input.is_empty() {
            return default.to_string();
        } else {
            if prompt == "Enter RPC port" {
                if trimmed_input.parse::<u16>().is_ok() {
                    return trimmed_input.to_string();
                } else {
                    println!("Invalid port number. Please enter a valid u16 value.");
                    continue;
                }
            }
            return trimmed_input.to_string();
        }
    }
}

/// Complete network configuration setup with user prompts
pub fn setup_network_config(chain_id: &str) -> io::Result<NetworkConfig> {
    // Ensure kanari.yaml exists
    init_default_config()?;

    // Check if network configuration already exists
    if let Some(existing_config) = get_network_config()? {
        println!("Configuration already exists. Skipping configuration process.");
        return Ok(existing_config);
    }

    // Prompt for new configuration
    let default_rpc_port = "30030";
    let rpc_port = prompt_for_value("Enter RPC port", default_rpc_port)
        .parse::<u16>()
        .expect("Invalid port number");

    let localhost_only_str = prompt_for_value("Restrict to localhost only? (true/false)", "false");
    let localhost_only = localhost_only_str.to_lowercase() == "true";

    let use_tls_str = prompt_for_value(
        "Use TLS encryption for P2P communication? (true/false)",
        "false",
    );
    let use_tls = use_tls_str.to_lowercase() == "true";

    let network_config =
        configure_network_settings(chain_id, localhost_only, use_tls, Some(rpc_port))?;

    println!("Network configuration saved successfully.");
    Ok(network_config)
}

/// Get or create network configuration (unified entry point)
pub fn ensure_network_config(
    chain_id: &str,
    port: Option<u16>,
    localhost_only: bool,
    use_tls: bool,
    peers: Vec<String>,
) -> io::Result<NetworkConfig> {
    // Ensure kanari.yaml exists
    init_default_config()?;

    let mut network_config = if let Some(mut existing_config) = get_network_config()? {
        println!("Using existing network configuration.");

        // Always update chain_id if it's empty or incorrect
        if existing_config.chain_id.is_empty() || existing_config.chain_id == "kanari-local" {
            existing_config.chain_id = chain_id.to_string();
            println!("Updated chain_id to: {}", chain_id);
        }

        existing_config
    } else {
        // Create default network config without prompts
        configure_network_settings(chain_id, localhost_only, use_tls, port)?
    };

    // Update with provided parameters
    if let Some(p) = port {
        network_config.port = p;
    }

    network_config.localhost_only = localhost_only;
    network_config.use_tls = use_tls;
    network_config.peers = if localhost_only { vec![] } else { peers };

    // Ensure chain_id is always set correctly
    if network_config.chain_id.is_empty() || network_config.chain_id == "kanari-local" {
        network_config.chain_id = chain_id.to_string();
    }

    // Always save updated configuration to fix any issues
    let mut kanari_config = load_kanari_config()?;
    if let Some(kanari_mapping) = kanari_config.as_mapping_mut() {
        kanari_mapping.insert(
            Value::String("chain_id".to_string()),
            Value::String(network_config.chain_id.clone()),
        );
        kanari_mapping.insert(
            Value::String("localhost_only".to_string()),
            Value::Bool(localhost_only),
        );
        kanari_mapping.insert(Value::String("use_tls".to_string()), Value::Bool(use_tls));
        kanari_mapping.insert(
            Value::String("rpc_port".to_string()),
            Value::Number(serde_yaml::Number::from(network_config.port as u64)),
        );

        if !network_config.peers.is_empty() {
            let peers_yaml: Vec<Value> = network_config
                .peers
                .iter()
                .map(|p| Value::String(p.clone()))
                .collect();
            kanari_mapping.insert(
                Value::String("peers".to_string()),
                Value::Sequence(peers_yaml),
            );
        }

        save_kanari_config(&kanari_config)?;
        println!("Configuration updated and saved.");
    }

    Ok(network_config)
}
