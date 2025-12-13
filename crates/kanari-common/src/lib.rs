use serde_yaml::{Mapping, Value};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

// Path utility functions
pub fn get_kanari_dir() -> PathBuf {
    let mut path = dirs::home_dir().expect("Unable to find home directory");
    path.push(".kanari");
    fs::create_dir_all(&path).expect("Unable to create .kanari directory");
    path
}

/// Get path to the kanari.yaml configuration file
pub fn get_kanari_config_path() -> PathBuf {
    let mut config_dir = get_kanari_dir();
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

    if envs
        .iter()
        .any(|env| env.get("alias").and_then(|v| v.as_str()) == Some(active_env_str))
    {
        let mut config_map = Mapping::new();

        // `chain_id` removed from generated config_map as requested.

        if let Some(addr) = kanari_config.get("active_address").and_then(|v| v.as_str()) {
            config_map.insert(
                Value::String("address".to_string()),
                Value::String(addr.to_string()),
            );
        }

        // `rpc_port` removed from generated config_map as requested.

        return Ok(Value::Mapping(config_map));
    }

    Ok(Value::Mapping(Mapping::new())) // Active environment not found in envs list
}

/// Save configuration to kanari.yaml file
pub fn save_config(config_to_save: &Value) -> io::Result<()> {
    let mut kanari_config = load_kanari_config().unwrap_or_else(|_| Value::Mapping(Mapping::new()));

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

        // RPC updates removed per configuration changes.

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
