use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::process::ExitCode;
use std::str::FromStr;

use cas_secret_store::{CredentialId, SecretStoreError, read};

const EXIT_INVALID_ARGUMENTS: u8 = 2;
const EXIT_NOT_FOUND: u8 = 3;
const EXIT_STORE_UNAVAILABLE: u8 = 4;
const EXIT_PERMISSION_DENIED: u8 = 5;
const EXIT_RETRIEVAL_FAILED: u8 = 6;

fn main() -> ExitCode {
    match parse_args(env::args_os()) {
        Ok(id) => deliver(id),
        Err(()) => {
            eprintln!("Usage: cas-helper token <credential-id>");
            ExitCode::from(EXIT_INVALID_ARGUMENTS)
        }
    }
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<CredentialId, ()> {
    let mut args = args.into_iter();
    let _program = args.next();
    let command = args.next().ok_or(())?;
    let credential_id = args.next().ok_or(())?;

    if command != OsStr::new("token") || args.next().is_some() {
        return Err(());
    }

    CredentialId::from_str(credential_id.to_str().ok_or(())?).map_err(|_| ())
}

fn deliver(id: CredentialId) -> ExitCode {
    match read(id) {
        Ok(secret) => {
            let mut stdout = io::stdout().lock();
            if stdout.write_all(secret.expose()).is_err()
                || stdout.write_all(b"\n").is_err()
                || stdout.flush().is_err()
            {
                eprintln!("Credential output failed.");
                return ExitCode::from(EXIT_RETRIEVAL_FAILED);
            }
            ExitCode::SUCCESS
        }
        Err(SecretStoreError::NotFound) => {
            eprintln!("Credential not found.");
            ExitCode::from(EXIT_NOT_FOUND)
        }
        Err(SecretStoreError::Unavailable) => {
            eprintln!("Credential store unavailable.");
            ExitCode::from(EXIT_STORE_UNAVAILABLE)
        }
        Err(SecretStoreError::AccessDenied) => {
            eprintln!("Credential access denied.");
            ExitCode::from(EXIT_PERMISSION_DENIED)
        }
        Err(_) => {
            eprintln!("Credential retrieval failed.");
            ExitCode::from(EXIT_RETRIEVAL_FAILED)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_token_and_uuid() {
        let valid = [
            OsString::from("cas-helper"),
            OsString::from("token"),
            OsString::from("0198ae47-1234-5678-9abc-0123456789ef"),
        ];
        let injected = [
            OsString::from("cas-helper"),
            OsString::from("token"),
            OsString::from("../../secret"),
        ];

        assert!(parse_args(valid).is_ok());
        assert!(parse_args(injected).is_err());
    }
}
