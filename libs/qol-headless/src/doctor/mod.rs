mod check;
mod contract;
mod render;

pub use check::DoctorCheck;
pub use contract::{
    DoctorAggregateReport, DoctorCheckResult, DoctorReport, DoctorStatus, PluginDoctorReport,
    PreservedDoctorReport,
};
pub(crate) use render::{aggregate_exit_code, render_aggregate_report, render_report};
