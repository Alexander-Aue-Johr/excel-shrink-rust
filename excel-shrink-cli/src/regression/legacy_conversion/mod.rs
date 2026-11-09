//! Compatibility entry points backed by the shared Excel automation crate.
pub(crate) use ::excel_automation::pool::{convert, Pool, write_seed};
pub(crate) mod performance {
    pub(crate) use ::excel_automation::pool::performance::*;
}
#[cfg(test)]
mod tests;
