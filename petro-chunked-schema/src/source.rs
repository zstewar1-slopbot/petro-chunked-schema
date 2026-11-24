//! URI-like references (`builtin:` and `file:`) for locating schema definitions.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::SchemaRefError;

/// Location of a YAML schema definition, supporting both embedded built-in schemas and local
/// filesystem paths.
///
/// # URI Scheme Rules
/// - `builtin:<Name>` refers to an embedded schema registered in [`crate::known_formats`] (for
///   example `builtin:Alo`, `builtin:AloModel`, `builtin:AloParticle`, `builtin:AlaAnimation`).
/// - `file:<Path>` explicitly forces interpretation as a local file path, allowing paths whose
///   names literally begin with `builtin:` or `file:` (for example `file:builtin:foo`).
/// - Any string without a `builtin:` or `file:` prefix defaults to [`SchemaRef::File`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SchemaRef {
    /// Embedded built-in format name (without the `builtin:` prefix).
    Builtin(String),
    /// Path to a YAML schema file on disk.
    File(PathBuf),
}

impl SchemaRef {
    /// Parses a [`SchemaRef`] from its URI or path string representation.
    pub fn parse(input: &str) -> Result<Self, SchemaRefError> {
        if input.is_empty() {
            return Err(SchemaRefError::Empty);
        }
        if let Some(name) = input.strip_prefix("builtin:") {
            if name.is_empty() {
                return Err(SchemaRefError::EmptyBuiltinName);
            }
            return Ok(Self::Builtin(name.to_owned()));
        }
        if let Some(path) = input.strip_prefix("file:") {
            if path.is_empty() {
                return Err(SchemaRefError::EmptyFilePath);
            }
            return Ok(Self::File(PathBuf::from(path)));
        }
        Ok(Self::File(PathBuf::from(input)))
    }

    /// Returns `true` if this reference targets an embedded built-in schema.
    pub fn is_builtin(&self) -> bool {
        matches!(self, Self::Builtin(_))
    }

    /// Returns the built-in format name if this is [`SchemaRef::Builtin`].
    pub fn as_builtin(&self) -> Option<&str> {
        match self {
            Self::Builtin(name) => Some(name.as_str()),
            Self::File(_) => None,
        }
    }

    /// Returns the filesystem path if this is [`SchemaRef::File`].
    pub fn as_file(&self) -> Option<&Path> {
        match self {
            Self::Builtin(_) => None,
            Self::File(path) => Some(path.as_path()),
        }
    }

    /// Resolves relative [`SchemaRef::File`] paths against `base` when `base` is also a file
    /// reference; built-in and absolute references are returned unchanged.
    pub fn resolve_relative(&self, base: Option<&SchemaRef>) -> Self {
        match (self, base) {
            (Self::File(path), Some(Self::File(base_path))) if path.is_relative() => {
                if let Some(parent) = base_path.parent() {
                    Self::File(parent.join(path))
                } else {
                    self.clone()
                }
            }
            _ => self.clone(),
        }
    }
}

impl FromStr for SchemaRef {
    type Err = SchemaRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for SchemaRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin(name) => write!(f, "builtin:{name}"),
            Self::File(path) => {
                let text = path.to_string_lossy();
                if text.starts_with("builtin:") || text.starts_with("file:") {
                    write!(f, "file:{text}")
                } else {
                    write!(f, "{text}")
                }
            }
        }
    }
}

impl Serialize for SchemaRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SchemaRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}
