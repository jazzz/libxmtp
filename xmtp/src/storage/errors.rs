use crate::contact::ContactError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Diesel connection error")]
    DieselConnectError(#[from] diesel::ConnectionError),
    #[error("Diesel result error: {0}")]
    DieselResultError(#[from] diesel::result::Error),
    #[error("Pool error {0}")]
    PoolError(String),
    #[error("Either incorrect encryptionkey or file is not a db {0}")]
    DbInitError(String),
    #[error("Store Error")]
    Store(String),
    #[error(transparent)]
    ImplementationError(#[from] anyhow::Error),
    #[error("ContactError")]
    ContactError(#[from] ContactError),
    #[error("serialization error")]
    SerializationError,
    #[error("unknown storage error: {0}")]
    Unknown(String),
}
