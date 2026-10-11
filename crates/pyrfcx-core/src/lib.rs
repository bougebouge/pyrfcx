#![allow(clippy::result_large_err)]

pub mod chunked;
pub mod connection;
pub mod describe;
pub mod error;
pub mod ffi;
pub(crate) mod marshal;
mod mem;
pub mod repository;
pub mod uc;
pub mod value;

pub use chunked::{ChunkedOutput, TableReader};
pub use connection::{
    CancelToken, CompatOptions, Connection, ConnectionAttributes, ConnectionParams,
};
pub use describe::{
    FieldDesc, FuncDesc, NamedFieldDesc, ParamDesc, RfcDirection, RfcFieldType, TypeInfo,
};
pub use error::{RfcError, RfcErrorInfo};
pub use value::RfcValue;
