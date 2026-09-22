//! Planning: from the repository and the settings to a `Plan` (a dry run changes nothing).

mod build;
mod entries;
mod pathplan;
mod preconditions;
mod range;
mod render;
mod timing;
mod types;

pub use build::build_plan;
pub use preconditions::check_preconditions;
pub use render::render;
pub use types::{Built, PlanOptions, RangeSpec};
