// URL and Display utilities for metadata
// Corresponds to `kanari_framework::url` and `kanari_framework::display` modules
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A URL type for web resources
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Url {
    pub url: String,
}

impl Url {
    /// Create a new URL
    pub fn new(url_bytes: Vec<u8>) -> Result<Self, UrlError> {
        let url = String::from_utf8(url_bytes)
            .map_err(|_| UrlError::InvalidUtf8)?;
        
        if !Self::is_valid_url(&url) {
            return Err(UrlError::InvalidUrl);
        }

        Ok(Self { url })
    }

    /// Create URL from string
    pub fn from_string(url: String) -> Result<Self, UrlError> {
        if !Self::is_valid_url(&url) {
            return Err(UrlError::InvalidUrl);
        }
        Ok(Self { url })
    }

    /// Get the inner URL string
    pub fn inner_url(&self) -> &str {
        &self.url
    }

    /// Update the URL
    pub fn update(&mut self, url: String) -> Result<(), UrlError> {
        if !Self::is_valid_url(&url) {
            return Err(UrlError::InvalidUrl);
        }
        self.url = url;
        Ok(())
    }

    /// Basic URL validation
    fn is_valid_url(url: &str) -> bool {
        url.starts_with("http://") || 
        url.starts_with("https://") || 
        url.starts_with("ipfs://") ||
        url.starts_with("data:")
    }

    /// Convert to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        self.url.as_bytes().to_vec()
    }
}

/// Display object for rich metadata presentation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Display<T> {
    pub id: crate::object::UID,
    pub fields: HashMap<String, String>,
    pub version: u16,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Display<T> {    /// Create a new display object
    pub fn new() -> Self {
        Self {
            id: crate::object::UID::new(crate::object::ID::new(crate::address::Address::zero())),
            fields: HashMap::new(),
            version: 1,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create display with publisher capability
    pub fn new_with_fields(fields: HashMap<String, String>) -> Self {
        Self {
            id: crate::object::UID::new(crate::object::ID::new(crate::address::Address::zero())),
            fields,
            version: 1,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Add a display field
    pub fn add(&mut self, name: String, value: String) {
        self.fields.insert(name, value);
    }

    /// Edit a display field
    pub fn edit(&mut self, name: String, value: String) {
        self.fields.insert(name, value);
    }

    /// Remove a display field
    pub fn remove(&mut self, name: &str) -> bool {
        self.fields.remove(name).is_some()
    }

    /// Get a field value
    pub fn get(&self, name: &str) -> Option<&String> {
        self.fields.get(name)
    }

    /// Check if field exists
    pub fn has_field(&self, name: &str) -> bool {
        self.fields.contains_key(name)
    }

    /// Get the version
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Update version
    pub fn update_version(&mut self, version: u16) {
        self.version = version;
    }

    /// Get all field names
    pub fn field_names(&self) -> Vec<String> {
        self.fields.keys().cloned().collect()
    }

    /// Get all fields as a reference
    pub fn fields(&self) -> &HashMap<String, String> {
        &self.fields
    }
}

impl<T> Default for Display<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Standard display field names
pub mod display_fields {
    pub const NAME: &str = "name";
    pub const DESCRIPTION: &str = "description";
    pub const LINK: &str = "link";
    pub const IMAGE_URL: &str = "image_url";
    pub const THUMBNAIL_URL: &str = "thumbnail_url";
    pub const PROJECT_URL: &str = "project_url";
    pub const CREATOR: &str = "creator";
}

/// Display template builder
pub struct DisplayBuilder<T> {
    display: Display<T>,
}

impl<T> DisplayBuilder<T> {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            display: Display::new(),
        }
    }

    /// Add name field
    pub fn with_name(mut self, name: String) -> Self {
        self.display.add(display_fields::NAME.to_string(), name);
        self
    }

    /// Add description field
    pub fn with_description(mut self, description: String) -> Self {
        self.display.add(display_fields::DESCRIPTION.to_string(), description);
        self
    }

    /// Add image URL field
    pub fn with_image_url(mut self, url: String) -> Self {
        self.display.add(display_fields::IMAGE_URL.to_string(), url);
        self
    }

    /// Add link field
    pub fn with_link(mut self, link: String) -> Self {
        self.display.add(display_fields::LINK.to_string(), link);
        self
    }

    /// Add custom field
    pub fn with_field(mut self, name: String, value: String) -> Self {
        self.display.add(name, value);
        self
    }

    /// Build the display object
    pub fn build(self) -> Display<T> {
        self.display
    }
}

impl<T> Default for DisplayBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// URL and Display errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    #[error("Invalid UTF-8 in URL")]
    InvalidUtf8,
    #[error("Invalid URL format")]
    InvalidUrl,
    #[error("URL too long")]
    UrlTooLong,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DisplayError {
    #[error("Field not found")]
    FieldNotFound,
    #[error("Invalid field name")]
    InvalidFieldName,
    #[error("Field value too long")]
    FieldValueTooLong,
}

pub type UrlResult<T> = Result<T, UrlError>;
pub type DisplayResult<T> = Result<T, DisplayError>;
