#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(not(windows))]
compile_error!("cas-secret-store currently supports Windows only");

use std::fmt;
use std::ptr;
use std::str::FromStr;
use std::sync::atomic::{Ordering, compiler_fence};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_NO_SUCH_LOGON_SESSION, ERROR_NOT_FOUND, GetLastError,
};
use windows_sys::Win32::Security::Credentials::{
    CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW,
    CredDeleteW, CredFree, CredReadW, CredWriteW,
};

const TARGET_PREFIX: &str = "CodexAgentSwitch:credential:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialId([u8; 16]);

impl FromStr for CredentialId {
    type Err = SecretStoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let input = value.as_bytes();
        if input.len() != 36 || [8, 13, 18, 23].iter().any(|index| input[*index] != b'-') {
            return Err(SecretStoreError::InvalidCredentialId);
        }

        let mut bytes = [0_u8; 16];
        let mut output_index = 0;
        let mut high_nibble = None;

        for (index, byte) in input.iter().copied().enumerate() {
            if matches!(index, 8 | 13 | 18 | 23) {
                continue;
            }

            let nibble = hex_value(byte).ok_or(SecretStoreError::InvalidCredentialId)?;
            if let Some(high) = high_nibble.take() {
                bytes[output_index] = (high << 4) | nibble;
                output_index += 1;
            } else {
                high_nibble = Some(nibble);
            }
        }

        (output_index == 16 && high_nibble.is_none())
            .then_some(Self(bytes))
            .ok_or(SecretStoreError::InvalidCredentialId)
    }
}

impl fmt::Display for CredentialId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                formatter.write_str("-")?;
            }
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

pub struct SecretValue(Vec<u8>);

impl SecretValue {
    pub fn from_string(value: String) -> Result<Self, SecretStoreError> {
        Self::from_bytes(value.into_bytes())
    }

    fn from_bytes(bytes: Vec<u8>) -> Result<Self, SecretStoreError> {
        let value = Self(bytes);
        let valid = !value.0.is_empty()
            && value.0.len() <= CRED_MAX_CREDENTIAL_BLOB_SIZE as usize
            && !value.0.iter().any(|byte| matches!(byte, 0 | b'\r' | b'\n'));

        valid
            .then_some(value)
            .ok_or(SecretStoreError::InvalidSecret)
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            // SAFETY: byte points to a live element exclusively borrowed from this Vec.
            unsafe { ptr::write_volatile(byte, 0) };
        }
        compiler_fence(Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStoreError {
    InvalidCredentialId,
    InvalidSecret,
    NotFound,
    AccessDenied,
    Unavailable,
    ReadFailed(u32),
    WriteFailed(u32),
    DeleteFailed(u32),
}

impl fmt::Display for SecretStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredentialId => formatter.write_str("invalid credential id"),
            Self::InvalidSecret => formatter.write_str("invalid secret"),
            Self::NotFound => formatter.write_str("credential not found"),
            Self::AccessDenied => formatter.write_str("credential access denied"),
            Self::Unavailable => formatter.write_str("credential store unavailable"),
            Self::ReadFailed(code) => write!(formatter, "credential read failed ({code})"),
            Self::WriteFailed(code) => write!(formatter, "credential write failed ({code})"),
            Self::DeleteFailed(code) => write!(formatter, "credential delete failed ({code})"),
        }
    }
}

impl std::error::Error for SecretStoreError {}

pub fn store(id: CredentialId, secret: &SecretValue) -> Result<(), SecretStoreError> {
    let mut target = target_name(id);
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: secret.0.len() as u32,
        CredentialBlob: secret.0.as_ptr() as *mut u8,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };

    // SAFETY: all pointers remain valid for the duration of this synchronous Win32 call.
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        return Err(last_error(Operation::Write));
    }

    Ok(())
}

pub fn read(id: CredentialId) -> Result<SecretValue, SecretStoreError> {
    let target = target_name(id);
    let mut pointer = ptr::null_mut();

    // SAFETY: target is null-terminated and pointer is a valid out parameter.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } == 0 {
        return Err(last_error(Operation::Read));
    }

    let buffer = CredentialBuffer(pointer);
    // SAFETY: a successful CredReadW returns a valid CREDENTIALW until CredFree.
    let credential = unsafe { &*buffer.0 };
    if credential.CredentialBlob.is_null() && credential.CredentialBlobSize != 0 {
        return Err(SecretStoreError::ReadFailed(0));
    }

    // SAFETY: the blob is part of the CredReadW buffer and has the reported byte length.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
        .to_vec()
    };

    SecretValue::from_bytes(bytes)
}

pub fn exists(id: CredentialId) -> Result<bool, SecretStoreError> {
    let target = target_name(id);
    let mut pointer = ptr::null_mut();

    // SAFETY: target is null-terminated and pointer is a valid out parameter.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } != 0 {
        let _buffer = CredentialBuffer(pointer);
        return Ok(true);
    }

    match last_error(Operation::Read) {
        SecretStoreError::NotFound => Ok(false),
        error => Err(error),
    }
}

pub fn delete(id: CredentialId) -> Result<bool, SecretStoreError> {
    let target = target_name(id);

    // SAFETY: target is a valid null-terminated UTF-16 string.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
        return Ok(true);
    }

    match last_error(Operation::Delete) {
        SecretStoreError::NotFound => Ok(false),
        error => Err(error),
    }
}

struct CredentialBuffer(*mut CREDENTIALW);

impl Drop for CredentialBuffer {
    fn drop(&mut self) {
        if self.0.is_null() {
            return;
        }

        // SAFETY: this buffer came from CredReadW and is owned until CredFree.
        unsafe {
            let credential = &mut *self.0;
            if !credential.CredentialBlob.is_null() {
                for index in 0..credential.CredentialBlobSize as usize {
                    ptr::write_volatile(credential.CredentialBlob.add(index), 0);
                }
                compiler_fence(Ordering::SeqCst);
            }
            CredFree(self.0.cast());
        }
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Read,
    Write,
    Delete,
}

fn last_error(operation: Operation) -> SecretStoreError {
    // SAFETY: GetLastError has no preconditions and is called immediately after failure.
    let code = unsafe { GetLastError() };
    match code {
        ERROR_NOT_FOUND => SecretStoreError::NotFound,
        ERROR_ACCESS_DENIED => SecretStoreError::AccessDenied,
        ERROR_NO_SUCH_LOGON_SESSION => SecretStoreError::Unavailable,
        _ => match operation {
            Operation::Read => SecretStoreError::ReadFailed(code),
            Operation::Write => SecretStoreError::WriteFailed(code),
            Operation::Delete => SecretStoreError::DeleteFailed(code),
        },
    }
}

fn target_name(id: CredentialId) -> Vec<u16> {
    format!("{TARGET_PREFIX}{id}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn credential_id_is_strict_and_canonical() {
        let id = CredentialId::from_str("0198AE47-1234-5678-9ABC-0123456789EF").unwrap();

        assert_eq!(id.to_string(), "0198ae47-1234-5678-9abc-0123456789ef");
        assert!(CredentialId::from_str("../../secret").is_err());
        assert!(CredentialId::from_str("0198ae47:1234:5678:9abc:0123456789ef").is_err());
    }

    #[test]
    fn rejects_unsafe_secret_input() {
        assert!(SecretValue::from_string(String::new()).is_err());
        assert!(SecretValue::from_string("token\nleak".to_owned()).is_err());
        assert!(
            SecretValue::from_string("x".repeat(CRED_MAX_CREDENTIAL_BLOB_SIZE as usize + 1))
                .is_err()
        );
    }

    #[test]
    #[ignore = "writes a synthetic credential to the current Windows user store"]
    fn windows_credential_round_trip() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = CredentialId::from_str(&format!(
            "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
            unique as u32,
            (unique >> 32) as u16,
            (unique >> 48) as u16 & 0x0fff,
            std::process::id() & 0x0fff,
            (unique >> 64) as u64 & 0x0000_ffff_ffff_ffff,
        ))
        .unwrap();
        let _cleanup = Cleanup(id);
        let secret = SecretValue::from_string("CAS_TEST_SECRET_NOT_REAL".to_owned()).unwrap();

        store(id, &secret).unwrap();
        assert!(exists(id).unwrap());
        let stored = read(id).unwrap();
        assert_eq!(stored.expose(), b"CAS_TEST_SECRET_NOT_REAL");
        assert!(delete(id).unwrap());
        assert!(!exists(id).unwrap());
    }

    struct Cleanup(CredentialId);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = delete(self.0);
        }
    }
}
