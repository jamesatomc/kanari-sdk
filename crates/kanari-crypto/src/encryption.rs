// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Secure data encryption and decryption
//!
//! This module provides functions for encrypting and decrypting data using
//!
//! **Classical**: AES-256-GCM with Argon2 key derivation

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use argon2::{
    Algorithm, Argon2, Version,
    password_hash::{PasswordHasher, SaltString},
};
use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize};
// OsRng from bip39's rand (v0.8) is compatible with password-hash's rand_core v0.6
use bip39::rand::rngs::OsRng;
use std::fmt;
use std::io::{self, ErrorKind, Read, Write};
use std::string::ToString;
use thiserror::Error;
use zeroize::Zeroize;

/// Encryption scheme selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EncryptionScheme {
    /// AES-256-GCM (Classical, fast)
    #[default]
    Aes256Gcm,
}

impl EncryptionScheme {
    /// Returns true if this scheme is quantum-resistant
    pub fn is_quantum_resistant(&self) -> bool {
        false
    }

    /// Get security level (1-5)
    pub fn security_level(&self) -> u8 {
        match self {
            EncryptionScheme::Aes256Gcm => 4,
        }
    }

    /// Check if this scheme is available (compile-time feature check)
    pub fn is_available(&self) -> bool {
        match self {
            EncryptionScheme::Aes256Gcm => true,
        }
    }
}

/// Error types for encryption operations
#[derive(Error, Debug)]
pub enum EncryptionError {
    #[error("Encryption error: {0}")]
    AeadError(String),

    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),

    #[error("Invalid format error: {0}")]
    InvalidFormat(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Decryption error")]
    DecryptionError,

    #[error("Post-quantum encryption error: {0}")]
    PqcError(String),

    #[error("Feature not available: {0} requires 'pqc' feature to be enabled")]
    FeatureNotAvailable(String),
}

/// Structure representing encrypted data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ciphertext_array: Vec<u8>,

    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    ciphertext: String,

    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nonce_array: Vec<u8>,

    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    nonce: String,

    salt: String,
}

/// Metadata required to decrypt an AES-256-GCM STREAM payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEncryptionHeader {
    pub format_version: u32,
    pub algorithm: String,
    pub salt: String,
    pub nonce: String,
    pub chunk_size: u32,
}

const STREAM_ENCRYPTION_FORMAT_VERSION: u32 = 1;
const STREAM_ENCRYPTION_ALGORITHM: &str = "AES-256-GCM-FRAMED-ARGON2ID";
const STREAM_NONCE_PREFIX_LEN: usize = 4;
const AEAD_TAG_LEN: usize = 16;
const STREAM_FRAME_AAD_LEN: usize =
    STREAM_ENCRYPTION_ALGORITHM.len() + STREAM_NONCE_PREFIX_LEN + 8 + 1 + 4;
pub const DEFAULT_STREAM_CHUNK_SIZE: usize = 1024 * 1024;
pub const MAX_STREAM_CHUNK_SIZE: usize = 16 * 1024 * 1024;

impl EncryptedData {
    /// Get the ciphertext bytes, regardless of format
    pub fn get_ciphertext(&self) -> Result<Vec<u8>, EncryptionError> {
        if !self.ciphertext.is_empty() {
            general_purpose::STANDARD
                .decode(&self.ciphertext)
                .map_err(|e| {
                    EncryptionError::InvalidFormat(format!("Invalid ciphertext base64: {}", e))
                })
        } else if !self.ciphertext_array.is_empty() {
            Ok(self.ciphertext_array.clone())
        } else {
            Err(EncryptionError::InvalidFormat(
                "Empty ciphertext".to_string(),
            ))
        }
    }

    /// Get the nonce bytes, regardless of format
    pub fn get_nonce(&self) -> Result<Vec<u8>, EncryptionError> {
        if !self.nonce.is_empty() {
            general_purpose::STANDARD
                .decode(&self.nonce)
                .map_err(|e| EncryptionError::InvalidFormat(format!("Invalid nonce base64: {}", e)))
        } else if !self.nonce_array.is_empty() {
            Ok(self.nonce_array.clone())
        } else {
            Err(EncryptionError::InvalidFormat("Empty nonce".to_string()))
        }
    }
}

#[cfg(test)]
pub fn make_invalid_encrypted_data_for_tests() -> EncryptedData {
    EncryptedData {
        ciphertext_array: Vec::new(),
        ciphertext: String::new(),
        nonce_array: Vec::new(),
        nonce: String::new(),
        salt: "invalid_salt".to_string(),
    }
}

impl fmt::Display for EncryptedData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cipher_len = if !self.ciphertext.is_empty() {
            self.ciphertext.len()
        } else {
            self.ciphertext_array.len()
        };

        let nonce_len = if !self.nonce.is_empty() {
            self.nonce.len()
        } else {
            self.nonce_array.len()
        };

        write!(
            f,
            "EncryptedData {{ ciphertext: [{}], nonce: [{}], salt: {} }}",
            cipher_len, nonce_len, self.salt
        )
    }
}

/// Encrypt data with a password
pub fn encrypt_data(data: &[u8], password: &str) -> Result<EncryptedData, EncryptionError> {
    validate_password(password)?;

    // Generate a random salt for key derivation
    let salt = SaltString::generate(&mut OsRng);

    let key_bytes_vec = derive_key(password, &salt)?;
    let key_owned =
        Key::<Aes256Gcm>::try_from(key_bytes_vec.as_slice()).expect("Argon2 produces 32-byte key");

    // Generate a random nonce for AES-GCM
    let nonce_arr: [u8; 12] = rand::random();
    let nonce = Nonce::try_from(&nonce_arr[..]).expect("nonce is exactly 12 bytes");

    // Create the cipher for encryption
    let cipher = Aes256Gcm::new(&key_owned);

    // Encrypt the data
    let ciphertext = cipher
        .encrypt(&nonce, data)
        .map_err(|e| EncryptionError::AeadError(e.to_string()))?;

    // Store values in a more compact base64 representation
    let ciphertext_b64 = general_purpose::STANDARD.encode(&ciphertext);
    let nonce_b64 = general_purpose::STANDARD.encode(nonce_arr);

    // Zeroize intermediate derived key material as soon as possible
    drop(key_bytes_vec);

    Ok(EncryptedData {
        ciphertext_array: Vec::new(),
        ciphertext: ciphertext_b64,
        nonce_array: Vec::new(),
        nonce: nonce_b64,
        salt: salt.to_string(),
    })
}

/// Decrypt data with a password
pub fn decrypt_data(encrypted: &EncryptedData, password: &str) -> Result<Vec<u8>, EncryptionError> {
    validate_password(password)?;

    // Validate ciphertext size to prevent memory exhaustion attacks
    const MAX_CIPHERTEXT_SIZE: usize = 100 * 1024 * 1024; // 100MB
    // Decode ciphertext first (handles base64 or raw array) then check size in bytes
    let ciphertext = encrypted.get_ciphertext()?;
    if ciphertext.len() > MAX_CIPHERTEXT_SIZE {
        return Err(EncryptionError::InvalidFormat(
            "Ciphertext size exceeds maximum allowed".to_string(),
        ));
    }

    // Get salt from the encrypted data
    let salt = SaltString::from_b64(&encrypted.salt)
        .map_err(|_| EncryptionError::InvalidFormat("Invalid salt format".to_string()))?;

    let key_bytes_vec = derive_key(password, &salt)?;
    let key_owned =
        Key::<Aes256Gcm>::try_from(key_bytes_vec.as_slice()).expect("Argon2 produces 32-byte key");

    // We already decoded ciphertext above; get the nonce bytes now
    let nonce_bytes = encrypted.get_nonce()?;

    // Create nonce for decryption - need to convert Vec<u8> to Nonce
    if nonce_bytes.len() != 12 {
        return Err(EncryptionError::InvalidFormat(
            "Invalid nonce length".to_string(),
        ));
    }
    let nonce = aes_gcm::Nonce::try_from(nonce_bytes.as_slice())
        .expect("nonce length already validated as 12 bytes");

    // Create cipher for decryption (uses owned key)
    let cipher = Aes256Gcm::new(&key_owned);

    // Zeroize intermediate derived key material before decryption
    drop(key_bytes_vec);

    // Decrypt the data
    cipher
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|_| EncryptionError::DecryptionError)
}

/// Encrypt a reader into framed AES-256-GCM STREAM ciphertext.
///
/// The caller stores the returned header next to the ciphertext stream. The output
/// is a sequence of big-endian `u32` frame lengths followed by ciphertext frames.
pub fn encrypt_stream<R: Read, W: Write>(
    mut reader: R,
    writer: W,
    password: &str,
    chunk_size: usize,
) -> Result<StreamEncryptionHeader, EncryptionError> {
    let (header, mut writer) = stream_encrypting_writer(writer, password, chunk_size)?;
    io::copy(&mut reader, &mut writer)
        .map_err(|error| EncryptionError::AeadError(error.to_string()))?;
    writer.finish()?;
    Ok(header)
}

/// Decrypt framed AES-256-GCM STREAM ciphertext into a writer.
pub fn decrypt_stream<R: Read, W: Write>(
    header: &StreamEncryptionHeader,
    reader: R,
    mut writer: W,
    password: &str,
) -> Result<(), EncryptionError> {
    let mut reader = StreamDecryptingReader::new(header, reader, password)?;
    io::copy(&mut reader, &mut writer)
        .map_err(|error| EncryptionError::AeadError(error.to_string()))?;
    writer
        .flush()
        .map_err(|error| EncryptionError::AeadError(error.to_string()))?;
    Ok(())
}

/// A `Write` adapter that encrypts plaintext into framed AES-256-GCM stream output.
pub struct StreamEncryptingWriter<W: Write> {
    writer: W,
    cipher: Aes256Gcm,
    nonce_prefix: [u8; STREAM_NONCE_PREFIX_LEN],
    chunk_size: usize,
    buffer: Vec<u8>,
    frame_index: u64,
    finished: bool,
}

/// Build a streaming encrypting writer and the header required for decryption.
pub fn stream_encrypting_writer<W: Write>(
    writer: W,
    password: &str,
    chunk_size: usize,
) -> Result<(StreamEncryptionHeader, StreamEncryptingWriter<W>), EncryptionError> {
    validate_password(password)?;
    validate_chunk_size(chunk_size)?;

    let salt = SaltString::generate(&mut OsRng);
    let key_bytes_vec = derive_key(password, &salt)?;
    let cipher = cipher_from_derived(&key_bytes_vec);
    let nonce_prefix: [u8; STREAM_NONCE_PREFIX_LEN] = rand::random();
    drop(key_bytes_vec);

    Ok((
        StreamEncryptionHeader {
            format_version: STREAM_ENCRYPTION_FORMAT_VERSION,
            algorithm: STREAM_ENCRYPTION_ALGORITHM.to_string(),
            salt: salt.to_string(),
            nonce: general_purpose::STANDARD.encode(nonce_prefix),
            chunk_size: chunk_size as u32,
        },
        StreamEncryptingWriter {
            writer,
            cipher,
            nonce_prefix,
            chunk_size,
            buffer: Vec::with_capacity(chunk_size),
            frame_index: 0,
            finished: false,
        },
    ))
}

impl<W: Write> StreamEncryptingWriter<W> {
    pub fn finish(mut self) -> Result<W, EncryptionError> {
        if !self.finished {
            self.write_encrypted_frame(true)?;
            self.finished = true;
        }
        self.writer
            .flush()
            .map_err(|error| EncryptionError::AeadError(error.to_string()))?;
        Ok(self.writer)
    }

    fn write_encrypted_frame(&mut self, is_last: bool) -> Result<(), EncryptionError> {
        let aad = stream_frame_aad(
            &self.nonce_prefix,
            self.frame_index,
            is_last,
            self.buffer.len(),
        );
        let nonce_bytes = stream_frame_nonce(&self.nonce_prefix, self.frame_index);
        let nonce = Nonce::try_from(&nonce_bytes[..]).expect("stream nonce is exactly 12 bytes");
        let payload = Payload {
            msg: &self.buffer,
            aad: &aad,
        };
        let ciphertext = self
            .cipher
            .encrypt(&nonce, payload)
            .map_err(|error| EncryptionError::AeadError(error.to_string()))?;
        write_frame(&mut self.writer, is_last, &ciphertext)?;
        self.buffer.clear();
        if !is_last {
            self.frame_index = self.frame_index.checked_add(1).ok_or_else(|| {
                EncryptionError::InvalidInput("Encrypted stream frame counter overflow".to_string())
            })?;
        }
        Ok(())
    }
}

impl<W: Write> Write for StreamEncryptingWriter<W> {
    fn write(&mut self, mut buf: &[u8]) -> io::Result<usize> {
        if self.finished {
            return Err(io::Error::new(
                ErrorKind::BrokenPipe,
                "encrypted stream writer already finished",
            ));
        }
        let original_len = buf.len();
        while !buf.is_empty() {
            let available = self.chunk_size - self.buffer.len();
            let take = available.min(buf.len());
            self.buffer.extend_from_slice(&buf[..take]);
            buf = &buf[take..];
            if self.buffer.len() == self.chunk_size {
                self.write_encrypted_frame(false)
                    .map_err(encryption_error_to_io)?;
            }
        }
        Ok(original_len)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// A `Read` adapter that decrypts framed AES-256-GCM stream input on demand.
pub struct StreamDecryptingReader<R: Read> {
    reader: R,
    cipher: Aes256Gcm,
    nonce_prefix: [u8; STREAM_NONCE_PREFIX_LEN],
    chunk_size: usize,
    frame_index: u64,
    plaintext: Vec<u8>,
    offset: usize,
    finished_after_buffer: bool,
    ended: bool,
}

impl<R: Read> StreamDecryptingReader<R> {
    pub fn new(
        header: &StreamEncryptionHeader,
        reader: R,
        password: &str,
    ) -> Result<Self, EncryptionError> {
        validate_password(password)?;
        if header.format_version != STREAM_ENCRYPTION_FORMAT_VERSION {
            return Err(EncryptionError::InvalidFormat(format!(
                "Unsupported stream encryption format {}",
                header.format_version
            )));
        }
        if header.algorithm != STREAM_ENCRYPTION_ALGORITHM {
            return Err(EncryptionError::InvalidFormat(format!(
                "Unsupported stream encryption algorithm {}",
                header.algorithm
            )));
        }
        let chunk_size = header.chunk_size as usize;
        validate_chunk_size(chunk_size)?;

        let salt = SaltString::from_b64(&header.salt)
            .map_err(|_| EncryptionError::InvalidFormat("Invalid salt format".to_string()))?;
        let nonce_bytes = general_purpose::STANDARD
            .decode(&header.nonce)
            .map_err(|e| EncryptionError::InvalidFormat(format!("Invalid nonce base64: {}", e)))?;
        if nonce_bytes.len() != STREAM_NONCE_PREFIX_LEN {
            return Err(EncryptionError::InvalidFormat(
                "Invalid stream nonce length".to_string(),
            ));
        }
        let nonce_prefix: [u8; STREAM_NONCE_PREFIX_LEN] =
            nonce_bytes.as_slice().try_into().map_err(|_| {
                EncryptionError::InvalidFormat("Invalid stream nonce length".to_string())
            })?;

        let key_bytes_vec = derive_key(password, &salt)?;
        let cipher = cipher_from_derived(&key_bytes_vec);
        drop(key_bytes_vec);

        Ok(Self {
            reader,
            cipher,
            nonce_prefix,
            chunk_size,
            frame_index: 0,
            plaintext: Vec::new(),
            offset: 0,
            finished_after_buffer: false,
            ended: false,
        })
    }

    fn load_next_frame(&mut self) -> io::Result<()> {
        let Some((is_last, current)) = read_frame_optional(&mut self.reader, self.chunk_size)
            .map_err(encryption_error_to_io)?
        else {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "encrypted stream ended without final frame",
            ));
        };
        let plaintext_len = current.len().checked_sub(AEAD_TAG_LEN).ok_or_else(|| {
            encryption_error_to_io(EncryptionError::InvalidFormat(
                "Encrypted stream frame length is invalid".to_string(),
            ))
        })?;
        let aad = stream_frame_aad(&self.nonce_prefix, self.frame_index, is_last, plaintext_len);
        let nonce_bytes = stream_frame_nonce(&self.nonce_prefix, self.frame_index);
        let nonce = Nonce::try_from(&nonce_bytes[..]).expect("stream nonce is exactly 12 bytes");
        let payload = Payload {
            msg: &current,
            aad: &aad,
        };
        self.plaintext = self
            .cipher
            .decrypt(&nonce, payload)
            .map_err(|_| encryption_error_to_io(EncryptionError::DecryptionError))?;
        self.offset = 0;
        if is_last {
            if read_frame_optional(&mut self.reader, self.chunk_size)
                .map_err(encryption_error_to_io)?
                .is_some()
            {
                return Err(encryption_error_to_io(EncryptionError::InvalidFormat(
                    "Encrypted stream contains trailing data after final frame".to_string(),
                )));
            }
            self.finished_after_buffer = true;
        } else {
            self.frame_index = self.frame_index.checked_add(1).ok_or_else(|| {
                encryption_error_to_io(EncryptionError::InvalidInput(
                    "Encrypted stream frame counter overflow".to_string(),
                ))
            })?;
        }
        Ok(())
    }
}

impl<R: Read> Read for StreamDecryptingReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.offset == self.plaintext.len() {
            if self.finished_after_buffer {
                self.ended = true;
            }
            if self.ended {
                return Ok(0);
            }
            self.load_next_frame()?;
        }

        let available = &self.plaintext[self.offset..];
        let count = available.len().min(out.len());
        out[..count].copy_from_slice(&available[..count]);
        self.offset += count;
        Ok(count)
    }
}

fn validate_password(password: &str) -> Result<(), EncryptionError> {
    if password.is_empty() {
        return Err(EncryptionError::InvalidInput(
            "Password cannot be empty".to_string(),
        ));
    }
    if password.len() > crate::MAX_PASSWORD_LEN {
        return Err(EncryptionError::InvalidInput(format!(
            "Password exceeds maximum length of {} bytes",
            crate::MAX_PASSWORD_LEN
        )));
    }
    Ok(())
}

fn derive_key(
    password: &str,
    salt: &SaltString,
) -> Result<zeroize::Zeroizing<Vec<u8>>, EncryptionError> {
    let password_zero = zeroize::Zeroizing::new(password.as_bytes().to_vec());
    let params = argon2_params()?;
    let password_hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(&password_zero, salt)
        .map_err(|e| EncryptionError::KeyDerivationError(e.to_string()))?;

    let hash = password_hash.hash.ok_or_else(|| {
        EncryptionError::KeyDerivationError("Argon2 hash output is missing".to_string())
    })?;
    Ok(zeroize::Zeroizing::new(hash.as_bytes().to_vec()))
}

fn cipher_from_derived(key_bytes: &[u8]) -> Aes256Gcm {
    Aes256Gcm::new(&Key::<Aes256Gcm>::try_from(key_bytes).expect("Argon2 produces 32-byte key"))
}

fn encryption_error_to_io(error: EncryptionError) -> io::Error {
    let kind = match error {
        EncryptionError::InvalidFormat(_) | EncryptionError::DecryptionError => {
            ErrorKind::InvalidData
        }
        EncryptionError::InvalidInput(_) => ErrorKind::InvalidInput,
        _ => ErrorKind::Other,
    };
    io::Error::new(kind, error)
}

fn validate_chunk_size(chunk_size: usize) -> Result<(), EncryptionError> {
    if chunk_size == 0 {
        return Err(EncryptionError::InvalidInput(
            "Stream chunk size cannot be zero".to_string(),
        ));
    }
    if chunk_size > MAX_STREAM_CHUNK_SIZE {
        return Err(EncryptionError::InvalidInput(format!(
            "Stream chunk size exceeds maximum allowed {} bytes",
            MAX_STREAM_CHUNK_SIZE
        )));
    }
    if chunk_size > u32::MAX as usize - AEAD_TAG_LEN {
        return Err(EncryptionError::InvalidInput(
            "Stream chunk size is too large".to_string(),
        ));
    }
    Ok(())
}

fn stream_frame_nonce(prefix: &[u8; STREAM_NONCE_PREFIX_LEN], frame_index: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[..STREAM_NONCE_PREFIX_LEN].copy_from_slice(prefix);
    nonce[STREAM_NONCE_PREFIX_LEN..].copy_from_slice(&frame_index.to_be_bytes());
    nonce
}

fn stream_frame_aad(
    prefix: &[u8; STREAM_NONCE_PREFIX_LEN],
    frame_index: u64,
    is_last: bool,
    plaintext_len: usize,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(STREAM_FRAME_AAD_LEN);
    aad.extend_from_slice(STREAM_ENCRYPTION_ALGORITHM.as_bytes());
    aad.extend_from_slice(prefix);
    aad.extend_from_slice(&frame_index.to_be_bytes());
    aad.push(u8::from(is_last));
    aad.extend_from_slice(&(plaintext_len as u32).to_be_bytes());
    aad
}

fn write_frame<W: Write>(
    writer: &mut W,
    is_last: bool,
    frame: &[u8],
) -> Result<(), EncryptionError> {
    let frame_len = u32::try_from(frame.len()).map_err(|_| {
        EncryptionError::InvalidInput("Encrypted stream frame is too large".to_string())
    })?;
    writer
        .write_all(&[u8::from(is_last)])
        .and_then(|_| writer.write_all(&frame_len.to_be_bytes()))
        .and_then(|_| writer.write_all(frame))
        .map_err(|error| EncryptionError::AeadError(error.to_string()))
}

fn read_frame_optional<R: Read>(
    reader: &mut R,
    chunk_size: usize,
) -> Result<Option<(bool, Vec<u8>)>, EncryptionError> {
    let mut final_flag = [0u8; 1];
    match reader.read_exact(&mut final_flag) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(EncryptionError::AeadError(error.to_string())),
    }
    if final_flag[0] > 1 {
        return Err(EncryptionError::InvalidFormat(
            "Encrypted stream final-frame flag is invalid".to_string(),
        ));
    }
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            EncryptionError::InvalidFormat("Encrypted stream frame header is truncated".to_string())
        } else {
            EncryptionError::AeadError(error.to_string())
        }
    })?;
    let frame_len = u32::from_be_bytes(len_bytes) as usize;
    let max_frame_len = chunk_size.checked_add(AEAD_TAG_LEN).ok_or_else(|| {
        EncryptionError::InvalidInput("Stream chunk size is too large".to_string())
    })?;
    if frame_len > max_frame_len || frame_len < AEAD_TAG_LEN {
        return Err(EncryptionError::InvalidFormat(
            "Encrypted stream frame length is invalid".to_string(),
        ));
    }
    let mut frame = vec![0u8; frame_len];
    reader.read_exact(&mut frame).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            EncryptionError::InvalidFormat("Encrypted stream frame is truncated".to_string())
        } else {
            EncryptionError::AeadError(error.to_string())
        }
    })?;
    Ok(Some((final_flag[0] == 1, frame)))
}

// Helper function to get consistent argon2 parameters
// Uses OWASP recommended parameters for interactive applications
fn argon2_params() -> Result<argon2::Params, EncryptionError> {
    argon2::Params::new(
        47104,    // Memory cost (46 MB) - OWASP minimum recommendation
        3,        // Time cost (3 iterations) - improved security
        1,        // Parallelism (1 thread)
        Some(32), // Produce 32-byte output to use directly as AES-256 key
    )
    .map_err(|e| EncryptionError::KeyDerivationError(format!("Invalid Argon2 parameters: {}", e)))
}

/// Upgrade legacy encrypted data to new base64 format
pub fn upgrade_encrypted_data(old_data: EncryptedData) -> EncryptedData {
    // Only upgrade if using older array format
    if !old_data.ciphertext_array.is_empty() && old_data.ciphertext.is_empty() {
        EncryptedData {
            ciphertext: general_purpose::STANDARD.encode(&old_data.ciphertext_array),
            ciphertext_array: Vec::new(),
            nonce: general_purpose::STANDARD.encode(&old_data.nonce_array),
            nonce_array: Vec::new(),
            salt: old_data.salt,
        }
    } else {
        old_data
    }
}

/// Encrypt a string with a password
pub fn encrypt_string(data: &str, password: &str) -> Result<EncryptedData, EncryptionError> {
    encrypt_data(data.as_bytes(), password)
}

/// Decrypt a string with a password
pub fn decrypt_string(
    encrypted: &EncryptedData,
    password: &str,
) -> Result<String, EncryptionError> {
    let bytes = decrypt_data(encrypted, password)?;
    String::from_utf8(bytes).map_err(|e| EncryptionError::InvalidFormat(e.to_string()))
}

/// Securely erase sensitive data from memory
/// Uses zeroize crate for secure memory clearing
pub fn secure_erase(data: &mut [u8]) {
    data.zeroize();
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // Bug #3: Memory Safety in secure_clear/secure_erase (Critical)
    // ============================================================================

    #[test]
    fn test_secure_erase_clears_memory() {
        let mut data = vec![0xAA; 100]; // Fill with pattern

        // Verify data is set
        assert!(
            data.iter().all(|&b| b == 0xAA),
            "Data should be initialized"
        );

        // Secure erase
        secure_erase(&mut data);

        // Verify data is zeroed
        assert!(
            data.iter().all(|&b| b == 0),
            "Data should be zeroed after secure_erase"
        );
    }

    #[test]
    fn test_secure_erase_empty_array() {
        let mut data = vec![];
        secure_erase(&mut data); // Should not panic
        assert_eq!(data.len(), 0);
    }

    #[test]
    fn test_secure_erase_large_data() {
        let mut data = vec![0xFF; 10_000];
        secure_erase(&mut data);
        assert!(
            data.iter().all(|&b| b == 0),
            "Large data should be fully zeroed"
        );
    }

    // ============================================================================
    // Bug #5: Weak Argon2 Parameters (High)
    // ============================================================================

    #[test]
    fn test_argon2_parameters_meet_owasp_standards() {
        let params = argon2_params().expect("Should create valid Argon2 params");

        // OWASP recommendations for interactive applications:
        // - Memory: minimum 46 MB (47104 KB)
        // - Iterations: minimum 1, recommended 2-3
        assert!(
            params.m_cost() >= 46000,
            "Memory cost should be at least 46 MB (OWASP standard), got {} KB",
            params.m_cost()
        );

        assert!(
            params.t_cost() >= 2,
            "Time cost should be at least 2 iterations, got {}",
            params.t_cost()
        );

        assert_eq!(
            params.p_cost(),
            1,
            "Parallelism should be 1 for compatibility"
        );
    }

    #[test]
    fn test_argon2_stronger_than_old_params() {
        // Old params: 19456 KB (19 MB), 2 iterations
        // New params: 47104 KB (46 MB), 3 iterations
        let params = argon2_params().unwrap();

        assert!(
            params.m_cost() > 19456,
            "Memory cost should be increased from old 19 MB"
        );
        assert!(
            params.t_cost() >= 2,
            "Time cost should be at least maintained"
        );
    }

    // ============================================================================
    // Encryption/Decryption Tests
    // ============================================================================

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let data = b"sensitive data";
        let password = "strong_password_123";

        // Encrypt
        let encrypted = encrypt_data(data, password).expect("Encryption should succeed");

        // Verify encrypted data structure
        assert!(
            !encrypted.ciphertext.is_empty(),
            "Ciphertext should not be empty"
        );
        assert!(!encrypted.nonce.is_empty(), "Nonce should not be empty");
        assert!(!encrypted.salt.is_empty(), "Salt should not be empty");

        // Decrypt
        let decrypted = decrypt_data(&encrypted, password).expect("Decryption should succeed");

        // Verify
        assert_eq!(decrypted, data, "Decrypted data should match original");
    }

    #[test]
    fn test_encrypt_string_decrypt_string() {
        let original = "Hello, World!";
        let password = "test_password";

        let encrypted =
            encrypt_string(original, password).expect("String encryption should succeed");
        let decrypted =
            decrypt_string(&encrypted, password).expect("String decryption should succeed");

        assert_eq!(decrypted, original, "Decrypted string should match");
    }

    #[test]
    fn test_decrypt_with_wrong_password_fails() {
        let data = b"secret";
        let correct_password = "password123";
        let wrong_password = "wrong_password";

        let encrypted = encrypt_data(data, correct_password).unwrap();
        let result = decrypt_data(&encrypted, wrong_password);

        assert!(
            result.is_err(),
            "Decryption with wrong password should fail"
        );
        assert!(matches!(
            result.unwrap_err(),
            EncryptionError::DecryptionError
        ));
    }

    #[test]
    fn test_encrypt_empty_data() {
        let data = b"";
        let password = "password";

        let encrypted = encrypt_data(data, password).expect("Should encrypt empty data");
        let decrypted = decrypt_data(&encrypted, password).expect("Should decrypt empty data");

        assert_eq!(decrypted, data, "Empty data roundtrip should work");
    }

    #[test]
    fn test_encrypt_large_data() {
        let data = vec![0x42; 1_000_000]; // 1 MB
        let password = "password";

        let encrypted = encrypt_data(&data, password).expect("Should encrypt large data");
        let decrypted = decrypt_data(&encrypted, password).expect("Should decrypt large data");

        assert_eq!(decrypted, data, "Large data roundtrip should work");
    }

    #[test]
    fn test_stream_encrypt_decrypt_roundtrip() {
        let data = (0..2_500_000)
            .map(|value| (value % 251) as u8)
            .collect::<Vec<_>>();
        let password = "stream_password";
        let mut encrypted = Vec::new();
        let header = encrypt_stream(data.as_slice(), &mut encrypted, password, 64 * 1024).unwrap();
        let mut decrypted = Vec::new();

        decrypt_stream(&header, encrypted.as_slice(), &mut decrypted, password).unwrap();

        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_stream_decrypt_rejects_corruption() {
        let data = b"streaming secret";
        let password = "stream_password";
        let mut encrypted = Vec::new();
        let header = encrypt_stream(data.as_slice(), &mut encrypted, password, 8).unwrap();
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0x01;

        let result = decrypt_stream(&header, encrypted.as_slice(), Vec::new(), password);

        assert!(result.is_err());
    }

    #[test]
    fn test_stream_decrypt_rejects_truncation() {
        let data = b"streaming secret";
        let password = "stream_password";
        let mut encrypted = Vec::new();
        let header = encrypt_stream(data.as_slice(), &mut encrypted, password, 8).unwrap();
        encrypted.truncate(encrypted.len() - 2);

        let result = decrypt_stream(&header, encrypted.as_slice(), Vec::new(), password);

        assert!(result.is_err());
    }

    #[test]
    fn test_stream_decrypt_rejects_oversized_chunk_header() {
        let password = "stream_password";
        let header = StreamEncryptionHeader {
            format_version: STREAM_ENCRYPTION_FORMAT_VERSION,
            algorithm: STREAM_ENCRYPTION_ALGORITHM.to_string(),
            salt: SaltString::generate(&mut OsRng).to_string(),
            nonce: general_purpose::STANDARD.encode([0u8; STREAM_NONCE_PREFIX_LEN]),
            chunk_size: (MAX_STREAM_CHUNK_SIZE as u32) + 1,
        };

        let result = StreamDecryptingReader::new(&header, io::empty(), password);

        assert!(matches!(result, Err(EncryptionError::InvalidInput(_))));
    }

    #[test]
    fn test_different_passwords_produce_different_ciphertexts() {
        let data = b"same data";
        let password1 = "password1";
        let password2 = "password2";

        let encrypted1 = encrypt_data(data, password1).unwrap();
        let encrypted2 = encrypt_data(data, password2).unwrap();

        // Ciphertexts should be different
        assert_ne!(
            encrypted1.ciphertext, encrypted2.ciphertext,
            "Different passwords should produce different ciphertexts"
        );
    }

    #[test]
    fn test_same_password_produces_different_ciphertexts() {
        // Due to random nonce and salt
        let data = b"same data";
        let password = "password";

        let encrypted1 = encrypt_data(data, password).unwrap();
        let encrypted2 = encrypt_data(data, password).unwrap();

        // Salts should be different
        assert_ne!(
            encrypted1.salt, encrypted2.salt,
            "Each encryption should use unique salt"
        );

        // Nonces should be different
        assert_ne!(
            encrypted1.nonce, encrypted2.nonce,
            "Each encryption should use unique nonce"
        );
    }

    #[test]
    fn test_encrypted_data_get_methods() {
        let data = b"test";
        let password = "password";

        let encrypted = encrypt_data(data, password).unwrap();

        // Test get_ciphertext
        let ciphertext = encrypted.get_ciphertext().expect("Should get ciphertext");
        assert!(!ciphertext.is_empty(), "Ciphertext should not be empty");

        // Test get_nonce
        let nonce = encrypted.get_nonce().expect("Should get nonce");
        assert_eq!(nonce.len(), 12, "Nonce should be 12 bytes for AES-GCM");
    }

    #[test]
    fn test_invalid_nonce_length() {
        let encrypted = EncryptedData {
            ciphertext: general_purpose::STANDARD.encode(b"data"),
            ciphertext_array: Vec::new(),
            nonce: general_purpose::STANDARD.encode(b"short"), // Invalid: should be 12 bytes
            nonce_array: Vec::new(),
            salt: "salt".to_string(),
        };

        let result = decrypt_data(&encrypted, "password");
        assert!(result.is_err(), "Invalid nonce length should fail");
    }

    #[test]
    fn test_upgrade_encrypted_data() {
        // Test upgrading from array format to base64 format
        let old_data = EncryptedData {
            ciphertext_array: vec![1, 2, 3, 4],
            ciphertext: String::new(),
            nonce_array: vec![5, 6, 7, 8],
            nonce: String::new(),
            salt: "salt".to_string(),
        };

        let upgraded = upgrade_encrypted_data(old_data);

        assert!(
            upgraded.ciphertext_array.is_empty(),
            "Array should be cleared"
        );
        assert!(
            !upgraded.ciphertext.is_empty(),
            "Base64 should be populated"
        );
        assert!(
            upgraded.nonce_array.is_empty(),
            "Nonce array should be cleared"
        );
        assert!(
            !upgraded.nonce.is_empty(),
            "Nonce base64 should be populated"
        );
    }

    #[test]
    fn test_encryption_scheme_properties() {
        assert!(!EncryptionScheme::Aes256Gcm.is_quantum_resistant());
        assert_eq!(EncryptionScheme::Aes256Gcm.security_level(), 4);
    }

    #[test]
    fn test_encrypted_data_display() {
        let data = b"test";
        let password = "password";
        let encrypted = encrypt_data(data, password).unwrap();

        let display = format!("{}", encrypted);
        assert!(
            display.contains("EncryptedData"),
            "Display should show type"
        );
        assert!(
            display.contains("ciphertext"),
            "Display should mention ciphertext"
        );
    }
}
