// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Framed streaming AES-256-GCM encryption/decryption.

use super::{
    AEAD_TAG_LEN, EncryptionError, STREAM_ENCRYPTION_ALGORITHM, STREAM_ENCRYPTION_FORMAT_VERSION,
    STREAM_FRAME_AAD_LEN, STREAM_NONCE_PREFIX_LEN, cipher_from_derived, derive_key,
    encryption_error_to_io, validate_password,
};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, Payload},
};
use argon2::password_hash::SaltString;
use base64::{Engine as _, engine::general_purpose};
use bip39::rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::io::{self, ErrorKind, Read, Write};

pub const DEFAULT_STREAM_CHUNK_SIZE: usize = 1024 * 1024;
pub const MAX_STREAM_CHUNK_SIZE: usize = 16 * 1024 * 1024;

/// Metadata required to decrypt an AES-256-GCM STREAM payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEncryptionHeader {
    pub format_version: u32,
    pub algorithm: String,
    pub salt: String,
    pub nonce: String,
    pub chunk_size: u32,
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
    let cipher = cipher_from_derived(&key_bytes_vec)?;
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
        let nonce = Nonce::try_from(&nonce_bytes[..]).map_err(|_| {
            EncryptionError::InvalidInput("Invalid stream nonce length".to_string())
        })?;
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
        let cipher = cipher_from_derived(&key_bytes_vec)?;
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
        let nonce = Nonce::try_from(&nonce_bytes[..]).map_err(|_| {
            encryption_error_to_io(EncryptionError::InvalidInput(
                "Invalid stream nonce length".to_string(),
            ))
        })?;
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
