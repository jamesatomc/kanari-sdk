// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

//! Kanari Framework
//! Core framework implementation for the Kanari blockchain
//!
//! This framework provides the core functionality for building and
//! interacting with the Kanari blockchain platform.

use std::collections::{HashMap, BTreeMap};
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::fmt;

use move_core_types::language_storage::ModuleId;
use move_core_types::move_resource::MoveResource;
use move_core_types::identifier::{IdentStr, Identifier};
use move_binary_format::CompiledModule;
use move_binary_format::errors::VMError;
use move_bytecode_verifier::verifier::verify_module_unmetered;

// Add TOML parsing support
use toml::Value;

// Add Move compiler dependencies
use move_compiler::Compiler;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::TypeTag;
use move_symbol_pool::Symbol;
use move_compiler::shared::NumericalAddress;
use serde::{Deserialize, Serialize};

// Import the ident_str macro
#[macro_use]
extern crate move_core_types;

#[derive(Debug)]
pub enum FrameworkError {
    IoError(io::Error),
    MoveCompileError(String),
    PackageNotFound(String),
    InvalidPackage(String),
    DependencyError(String),
    ModuleError(VMError),
    InvalidModule(String),
}

impl fmt::Display for FrameworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameworkError::IoError(e) => write!(f, "IO Error: {}", e),
            FrameworkError::MoveCompileError(s) => write!(f, "Move Compile Error: {}", s),
            FrameworkError::PackageNotFound(s) => write!(f, "Package Not Found: {}", s),
            FrameworkError::InvalidPackage(s) => write!(f, "Invalid Package: {}", s),
            FrameworkError::DependencyError(s) => write!(f, "Dependency Error: {}", s),
            FrameworkError::ModuleError(e) => write!(f, "Module Error: {}", e),
            FrameworkError::InvalidModule(s) => write!(f, "Invalid Module: {}", s),
        }
    }
}

impl From<io::Error> for FrameworkError {
    fn from(err: io::Error) -> Self {
        FrameworkError::IoError(err)
    }
}

impl From<VMError> for FrameworkError {
    fn from(err: VMError) -> Self {
        FrameworkError::ModuleError(err)
    }
}

/// Framework package types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageType {
    Stdlib,
    System,
    Framework,
}

/// Get the path to a package in the framework
fn get_package_path(package: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("packages");
    path.push(package);
    path
}

/// Get the path to the Move standard library
pub fn get_stdlib_path() -> PathBuf {
    get_package_path("move-stdlib")
}

/// Get the path to the Kanari system
pub fn get_kanari_system_path() -> PathBuf {
    get_package_path("kanari-system")
}

/// Get the path to the Kanari framework
pub fn get_framework_path() -> PathBuf {
    get_package_path("kanari-framework")
}

/// Package dependency information
#[derive(Debug, Clone)]
pub struct PackageDependency {
    pub name: String,
    pub version: String,
    pub path: Option<PathBuf>,
}

/// Framework package management
pub struct Package {
    pub package_type: PackageType,
    path: PathBuf,
    pub dependencies: HashMap<String, PackageDependency>,
}

impl Package {
    /// Create new package instance
    pub fn new(package_type: PackageType) -> Result<Self, FrameworkError> {
        let path = match package_type {
            PackageType::Stdlib => get_stdlib_path(),
            PackageType::System => get_kanari_system_path(),
            PackageType::Framework => get_framework_path(),
        };

        if !path.exists() {
            return Err(FrameworkError::PackageNotFound(
                path.to_string_lossy().into(),
            ));
        }

        Ok(Self {
            package_type,
            path,
            dependencies: HashMap::new(),
        })
    }

    /// Load package dependencies from Move.toml
    pub fn load_dependencies(&mut self) -> Result<(), FrameworkError> {
        let toml_path = self.path.join("Move.toml");
        let content = fs::read_to_string(&toml_path)
            .map_err(|_| FrameworkError::PackageNotFound("Move.toml not found".into()))?;
        
        // Parse TOML content
        let parsed_toml = content.parse::<Value>()
            .map_err(|e| FrameworkError::InvalidPackage(format!("Invalid TOML format: {}", e)))?;
        
        // Extract dependencies section
        if let Some(deps) = parsed_toml.get("dependencies").and_then(|d| d.as_table()) {
            for (name, value) in deps {
                match value {
                    Value::String(version) => {
                        self.dependencies.insert(name.clone(), PackageDependency {
                            name: name.clone(),
                            version: version.clone(),
                            path: None,
                        });
                    },
                    Value::Table(table) => {
                        let version = table.get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("0.0.0")
                            .to_string();
                        
                        let path = table.get("path")
                            .and_then(|v| v.as_str())
                            .map(|p| PathBuf::from(p));
                        
                        self.dependencies.insert(name.clone(), PackageDependency {
                            name: name.clone(),
                            version,
                            path,
                        });
                    },
                    _ => return Err(FrameworkError::InvalidPackage(
                        format!("Invalid dependency format for {}", name)
                    )),
                }
            }
        }
        
        Ok(())
    }

    /// Get package source files with metadata
    pub fn get_sources(&self) -> Result<Vec<(PathBuf, PackageSourceInfo)>, FrameworkError> {
        let mut sources = Vec::new();
        let source_dir = self.path.join("sources");

        for entry in fs::read_dir(source_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "move") {
                let info = PackageSourceInfo::create(&path)?;
                sources.push((path, info));
            }
        }

        Ok(sources)
    }

    /// Build package
    pub fn build(&self) -> Result<(), FrameworkError> {
        // Verify dependencies first
        self.verify_dependencies()?;

        // Get all source files
        let sources = self.get_sources()?;
        if sources.is_empty() {
            return Err(FrameworkError::InvalidPackage("No source files found".into()));
        }

        // Compile the sources into a ModuleStore
        let _module_store = self.compile()?;
        
        // At this point, compilation succeeded
        Ok(())
    }

    /// Verify package dependencies
    pub fn verify_dependencies(&self) -> Result<bool, FrameworkError> {
        let deps_path = self.path.join("Move.toml");
        if !deps_path.exists() {
            return Err(FrameworkError::PackageNotFound(
                "Move.toml not found".into(),
            ));
        }
        
        // Check if dependency paths exist
        for (name, dep) in &self.dependencies {
            if let Some(ref path) = dep.path {
                if !path.exists() {
                    return Err(FrameworkError::DependencyError(
                        format!("Dependency path for {} does not exist: {:?}", name, path)
                    ));
                }
            }
        }
        
        Ok(true)
    }

    /// Parse and verify a compiled Move module
    pub fn parse_module(&self, bytes: &[u8]) -> Result<CompiledModule, FrameworkError> {
        match CompiledModule::deserialize_with_defaults(bytes) {
            Ok(module) => {
                // Verify the module is valid
                verify_module_unmetered(&module).map_err(|e| FrameworkError::ModuleError(e))?;
                Ok(module)
            }
            Err(e) => Err(FrameworkError::InvalidModule(format!("Failed to deserialize module: {}", e))),
        }
    }

    /// Compile Move sources and return compiled modules
    pub fn compile(&self) -> Result<ModuleStore, FrameworkError> {
        let mut store = ModuleStore::new();
        
        // Get all source files
        let sources = self.get_sources()?;
        if sources.is_empty() {
            return Err(FrameworkError::InvalidPackage("No source files found".into()));
        }
        
        // Resolve dependency paths
        let dependency_paths = self.resolve_dependencies()?;
        
        // Convert PathBuf to String for the compiler
        let source_files: Vec<String> = sources.iter()
            .map(|(path, _)| path.to_string_lossy().to_string())
            .collect();
            
        // Convert dependency paths to String
        let dependency_paths: Vec<String> = dependency_paths.iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        
        // Set up addresses for the compiler - convert to BTreeMap<Symbol, NumericalAddress>
        let mut addresses = BTreeMap::new();
        // Add default addresses - adjust as needed for your ecosystem
        addresses.insert(
            Symbol::from("std"), 
            NumericalAddress::parse_str("0x1").unwrap()
        );
        // Add kanari_framework address to fix the "address with no value" errors
        addresses.insert(
            Symbol::from("kanari_framework"), 
            NumericalAddress::parse_str("0x2").unwrap()
        );
        
        println!("Compiling Move modules...");
        
        // Use from_files with correct argument order and add the missing vfs_root parameter
        let compiler = Compiler::from_files(
            None, // vfs_root parameter was missing
            source_files,
            dependency_paths,
            addresses,
        );
        
        match compiler.build_and_report() {
            Ok((_, compiled_units)) => {
                // Process compiled units - each is a NamedCompiledModule, not an enum
                for unit in compiled_units {
                    // Handle module (there's no need to match on variants since each unit is a module)
                    println!("Successfully compiled module: {}", unit.named_module.name());
                    
                    // Deserialize and verify the module
                    match self.parse_module(&unit.named_module.serialize(None)) {
                        Ok(compiled_module) => {
                            // Add the compiled module to our store
                            store.add_module(compiled_module)?;
                        }
                        Err(e) => {
                            return Err(FrameworkError::InvalidModule(
                                format!("Failed to deserialize module {}: {:?}", unit.named_module.name(), e)
                            ));
                        }
                    }
                }
                
                Ok(store)
            }
            Err(errors) => {
                // Format compilation errors
                let error_string = format!("Move compilation failed: {:?}", errors);
                Err(FrameworkError::MoveCompileError(error_string))
            }
        }
    }

    /// Resolve all dependencies for this package
    pub fn resolve_dependencies(&self) -> Result<Vec<PathBuf>, FrameworkError> {
        let mut dep_paths = Vec::new();
        
        // Add standard library if this isn't the stdlib itself
        if self.package_type != PackageType::Stdlib {
            dep_paths.push(get_stdlib_path());
        }
        
        // Add local dependencies
        for (_name, dep) in &self.dependencies {
            if let Some(ref path) = dep.path {
                dep_paths.push(path.clone());
            }
        }
        
        Ok(dep_paths)
    }
}

/// Module storage for compiled Move modules
pub struct ModuleStore {
    modules: HashMap<ModuleId, CompiledModule>,
}

impl ModuleStore {
    /// Create a new empty module store
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
        }
    }

    /// Add a compiled module to the store
    pub fn add_module(&mut self, module: CompiledModule) -> Result<(), FrameworkError> {
        let module_id = module.self_id();
        self.modules.insert(module_id, module);
        Ok(())
    }

    /// Get a module by its ID
    pub fn get_module(&self, id: &ModuleId) -> Option<&CompiledModule> {
        self.modules.get(id)
    }
}

/// Package source file information
#[derive(Debug)]
pub struct PackageSourceInfo {
    pub module_name: String,
    pub dependencies: Vec<String>,
    pub has_tests: bool,
}

impl PackageSourceInfo {
    // Keep the original implementation private
    fn new(path: &PathBuf) -> Result<Self, FrameworkError> {
        let content = fs::read_to_string(path)
            .map_err(|e| FrameworkError::IoError(e))?;
        
        // Extract module name from the file content directly
        // This is more reliable than using the file name
        let mut module_name = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
        
        // Parse the file to extract dependencies and check for tests
        let mut dependencies = Vec::new();
        let mut has_tests = false;
        let mut in_comment = false;
        let mut in_multiline_comment = false;
        
        // Improved parsing logic for Move source files
        for line in content.lines() {
            let line = line.trim();
            
            // Skip empty lines
            if line.is_empty() {
                continue;
            }
            
            // Handle comments
            if in_multiline_comment {
                if line.contains("*/") {
                    in_multiline_comment = false;
                }
                continue;
            }
            
            if in_comment {
                in_comment = false;
                continue;
            }
            
            if line.starts_with("//") {
                in_comment = true;
                continue;
            }
            
            if line.starts_with("/*") {
                in_multiline_comment = true;
                if line.contains("*/") {
                    in_multiline_comment = false;
                }
                continue;
            }
            
            // Extract module name from module declaration
            if line.starts_with("module ") {
                let parts: Vec<&str> = line.split("::").collect();
                if parts.len() >= 2 {
                    let mod_part = parts[1];
                    let mod_name = mod_part.split('{').next().unwrap_or(mod_part).trim();
                    if !mod_name.is_empty() {
                        module_name = mod_name.to_string();
                    }
                }
            }
            
            // Check for dependencies
            if line.starts_with("use ") {
                // Extract the dependency path more robustly
                let mut dep_line = line.trim_start_matches("use ");
                
                // Handle multi-line use statements
                if dep_line.ends_with(';') {
                    dep_line = dep_line.trim_end_matches(';');
                }
                
                // Remove any inline comments
                if let Some(comment_idx) = dep_line.find("//") {
                    dep_line = &dep_line[..comment_idx].trim();
                }
                
                // Handle use statements with curly braces
                if let Some(brace_idx) = dep_line.find('{') {
                    // For complex imports like "use std::{vector, option};"
                    let base_path = dep_line[..brace_idx].trim();
                    let items_part = &dep_line[brace_idx+1..];
                    if let Some(closing_idx) = items_part.find('}') {
                        let items = &items_part[..closing_idx];
                        for item in items.split(',') {
                            let item = item.trim();
                            if !item.is_empty() {
                                dependencies.push(format!("{}::{}", base_path, item));
                            }
                        }
                    }
                } else {
                    // Simple import
                    dependencies.push(dep_line.to_string());
                }
            }
            
            // Check for test functions with more precision
            if line.contains("#[test]") || line.contains("#[expected_failure]") {
                has_tests = true;
            }
        }
        
        Ok(Self {
            module_name,
            dependencies,
            has_tests,
        })
    }
    
    // Add a public wrapper for creating a PackageSourceInfo
    pub fn create(path: &PathBuf) -> Result<Self, FrameworkError> {
        Self::new(path)
    }
}

/// Extensions for working with Move resources
pub trait MoveResourceExt: MoveResource {
    /// Convert the resource to bytes
    fn to_bytes(&self) -> Vec<u8>;
    
    /// Create resource from bytes
    fn from_bytes(bytes: &[u8]) -> Result<Self, FrameworkError> where Self: Sized;
}

/// Example implementation of MoveResourceExt for a basic resource
#[derive(Serialize, Deserialize)]
pub struct SimpleResource {
    pub value: u64,
}

// First implement MoveStructType which is required by MoveResource
impl move_core_types::move_resource::MoveStructType for SimpleResource {
    const MODULE_NAME: &'static IdentStr = ident_str!("kanari_core"); 
    const STRUCT_NAME: &'static IdentStr = ident_str!("SimpleResource");
    
    const ADDRESS: AccountAddress = move_core_types::language_storage::CORE_CODE_ADDRESS;
    
    fn module_identifier() -> Identifier {
        Self::MODULE_NAME.to_owned()
    }
    
    fn struct_identifier() -> Identifier {
        Self::STRUCT_NAME.to_owned()
    }
    
    fn type_params() -> Vec<TypeTag> {
        std::vec![]
    }
    
    fn struct_tag() -> move_core_types::language_storage::StructTag {
        move_core_types::language_storage::StructTag {
            address: Self::ADDRESS,
            name: Self::struct_identifier(),
            module: Self::module_identifier(),
            type_params: Self::type_params(),
        }
    }
}

// Now we can implement MoveResource
impl MoveResource for SimpleResource {}

impl MoveResourceExt for SimpleResource {
    /// Convert the resource to bytes
    fn to_bytes(&self) -> Vec<u8> {
        bcs::to_bytes(self).unwrap_or_default()
    }
    
    /// Create resource from bytes
    fn from_bytes(bytes: &[u8]) -> Result<Self, FrameworkError> {
        bcs::from_bytes(bytes)
            .map_err(|e| FrameworkError::InvalidModule(format!("Failed to deserialize resource: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test the framework package management
    #[test]
    fn test_package_type() {
        // Test equality
        assert_eq!(PackageType::Stdlib, PackageType::Stdlib);
        assert_eq!(PackageType::System, PackageType::System);
        assert_eq!(PackageType::Framework, PackageType::Framework);
        
        assert_ne!(PackageType::Stdlib, PackageType::System);
        assert_ne!(PackageType::Stdlib, PackageType::Framework);
        assert_ne!(PackageType::System, PackageType::Framework);
        
        // Test copy semantics
        let stdlib = PackageType::Stdlib;
        let stdlib_copy = stdlib;
        assert_eq!(stdlib, stdlib_copy);
        
        // Test clone
        let framework = PackageType::Framework;
        let framework_clone = framework.clone();
        assert_eq!(framework, framework_clone);
        
        // Test debug formatting
        let debug_stdlib = format!("{:?}", PackageType::Stdlib);
        let debug_system = format!("{:?}", PackageType::System);
        let debug_framework = format!("{:?}", PackageType::Framework);
        
        assert_eq!(debug_stdlib, "Stdlib");
        assert_eq!(debug_system, "System");
        assert_eq!(debug_framework, "Framework");
    }

    /// Test the package management functionality
    #[test]
    fn test_package_build() {
        // Test Framework package build
        let framework_pkg = Package::new(PackageType::Framework).unwrap();
        let build_result = framework_pkg.build();
        assert!(build_result.is_ok(), "Framework build failed: {:?}", build_result.err());
        
        // Test Stdlib package build if available
        if get_stdlib_path().exists() {
            let stdlib_pkg = Package::new(PackageType::Stdlib).unwrap();
            let stdlib_build = stdlib_pkg.build();
            assert!(stdlib_build.is_ok(), "Stdlib build failed: {:?}", stdlib_build.err());
        }


        // Test resolve_dependencies
        let deps = framework_pkg.resolve_dependencies();
        assert!(deps.is_ok(), "Failed to resolve dependencies: {:?}", deps.err());
        let deps = deps.unwrap();
        assert!(!deps.is_empty(), "No dependencies resolved");
        
        // Test verify_dependencies
        let verify_result = framework_pkg.verify_dependencies();
        assert!(verify_result.is_ok(), "Failed to verify dependencies: {:?}", verify_result.err());
    }

    /// Test the package source information extraction
    #[test]
    fn test_simple_resource() {
        // Test with regular value
        let resource = SimpleResource { value: 42 };
        let bytes = resource.to_bytes();
        assert!(!bytes.is_empty(), "Serialized bytes should not be empty");
        
        let decoded = SimpleResource::from_bytes(&bytes).unwrap_or_else(|e| panic!("Failed to deserialize resource: {}", e));
        assert_eq!(resource.value, decoded.value, "Deserialized value does not match original");
        
        // Test with zero value
        let zero_resource = SimpleResource { value: 0 };
        let zero_bytes = zero_resource.to_bytes();
        let zero_decoded = SimpleResource::from_bytes(&zero_bytes).unwrap();
        assert_eq!(zero_resource.value, zero_decoded.value);
        
        // Test with max u64 value
        let max_resource = SimpleResource { value: u64::MAX };
        let max_bytes = max_resource.to_bytes();
        let max_decoded = SimpleResource::from_bytes(&max_bytes).unwrap();
        assert_eq!(max_resource.value, max_decoded.value);
        
        // Test error case with invalid data
        let invalid_data = [0, 1, 2]; // Too short to be a valid SimpleResource
        let invalid_result = SimpleResource::from_bytes(&invalid_data);
        assert!(invalid_result.is_err(), "Expected error for invalid data");
    }
}