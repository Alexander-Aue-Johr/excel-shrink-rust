//! Preparation adapters around the shared source-occurrence contract.
use anyhow::{ensure, Context, Result};
use std::{fs, path::Path};
pub(crate) use corpus_runner::sources::*;
pub(crate) mod package;
pub(crate) mod imported;
mod directory;
pub(crate) use directory::directory_inputs;
