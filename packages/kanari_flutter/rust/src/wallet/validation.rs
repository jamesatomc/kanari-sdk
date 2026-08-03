use crate::keys::{
    KANAHYBRID_PREFIX, KANAMLDSA_PREFIX, KANAPQC_PREFIX, KANARI_KEY_PREFIX, KANASLHDSA_PREFIX,
};

use super::WalletError;

pub(super) fn validate_wallet_secret_inputs(
    private_key: &str,
    password: &str,
) -> Result<(), WalletError> {
    validate_storage_password(password)?;

    if private_key.is_empty() {
        return Err(WalletError::EncryptionError(
            "Empty private key not allowed".to_string(),
        ));
    }

    Ok(())
}

pub(super) fn validate_storage_password(password: &str) -> Result<(), WalletError> {
    if password.is_empty() {
        return Err(WalletError::EncryptionError(
            "Empty password not allowed".to_string(),
        ));
    }

    if password.len() < crate::MIN_RECOMMENDED_PASSWORD_LENGTH {
        return Err(WalletError::EncryptionError(format!(
            "Password must be at least {} characters long",
            crate::MIN_RECOMMENDED_PASSWORD_LENGTH
        )));
    }

    if !crate::is_password_strong(password) {
        return Err(WalletError::EncryptionError(format!(
            "Password does not meet strength requirements ({}+ chars, mixed case, digits, special chars)",
            crate::MIN_RECOMMENDED_PASSWORD_LENGTH
        )));
    }

    Ok(())
}

pub(super) fn format_wallet_private_key(private_key: &str) -> String {
    if private_key.starts_with(KANARI_KEY_PREFIX)
        || private_key.starts_with(KANAMLDSA_PREFIX)
        || private_key.starts_with(KANASLHDSA_PREFIX)
        || private_key.starts_with(KANAPQC_PREFIX)
        || private_key.starts_with(KANAHYBRID_PREFIX)
    {
        private_key.to_string()
    } else {
        format!("{}{}", KANARI_KEY_PREFIX, private_key)
    }
}
