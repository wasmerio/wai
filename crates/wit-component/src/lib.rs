//! The WebAssembly component tooling.

#![deny(missing_docs)]

use anyhow::{bail, Result};
use std::str::FromStr;
use wasm_encoder::CanonicalOption;
use wasmer_wit_parser::Interface;

#[cfg(feature = "cli")]
pub mod cli;
mod decoding;
mod encoding;
mod printing;
mod validation;

pub use encoding::*;
pub use printing::*;

/// Supported string encoding formats.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum StringEncoding {
    /// Strings are encoded with UTF-8.
    UTF8,
    /// Strings are encoded with UTF-16.
    UTF16,
    /// Strings are encoded with compact UTF-16 (i.e. Latin1+UTF-16).
    CompactUTF16,
}

impl Default for StringEncoding {
    fn default() -> Self {
        StringEncoding::UTF8
    }
}

impl FromStr for StringEncoding {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "utf8" => Ok(StringEncoding::UTF8),
            "utf16" => Ok(StringEncoding::UTF16),
            "compact-utf16" => Ok(StringEncoding::CompactUTF16),
            _ => bail!("unknown string encoding `{}`", s),
        }
    }
}

impl From<StringEncoding> for wasm_encoder::CanonicalOption {
    fn from(e: StringEncoding) -> wasm_encoder::CanonicalOption {
        match e {
            StringEncoding::UTF8 => CanonicalOption::UTF8,
            StringEncoding::UTF16 => CanonicalOption::UTF16,
            StringEncoding::CompactUTF16 => CanonicalOption::CompactUTF16,
        }
    }
}

/// Decode an "interface-only" component to a wit `Interface`.
pub fn decode_interface_component(bytes: &[u8]) -> Result<Interface> {
    decoding::InterfaceDecoder::new(&decoding::ComponentInfo::new(bytes)?).decode()
}
