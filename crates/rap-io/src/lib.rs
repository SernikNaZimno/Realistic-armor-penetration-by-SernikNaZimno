//! Scenario files and result export.

pub mod scenario_file;
pub mod vtk;

use std::path::PathBuf;

use thiserror::Error;

pub use scenario_file::{FORMAT_VERSION, ScenarioFile};

/// Reasons a file cannot be read or written.
#[derive(Debug, Error)]
pub enum IoError {
    /// The file could not be read.
    #[error("cannot read {}: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The file could not be written.
    #[error("cannot write {}: {source}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The text is not a valid scenario.
    #[error("invalid scenario: {0}")]
    Parse(#[from] serde_json::Error),
    /// The file was written in a format this version does not understand.
    #[error(
        "scenario format version {found} is not supported; this program reads version {FORMAT_VERSION}"
    )]
    UnsupportedVersion {
        /// Version found in the file.
        found: u32,
    },
}
