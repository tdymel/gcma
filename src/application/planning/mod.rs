//! Planning: from the repository and the settings to a `Plan` (a dry run changes nothing).

mod build;
mod entries;
mod guards;
mod pathplan;
mod range;
mod timing;
mod types;

pub use build::build_plan;
pub use types::{Built, PlanOptions, RangeSpec};
