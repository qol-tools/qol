mod check;
mod contract;
mod device_permission;
mod render;

pub use check::DoctorCheck;
pub use contract::{
    DoctorAggregateReport, DoctorCheckResult, DoctorReport, DoctorStatus, PluginDoctorReport,
    PreservedDoctorReport,
};
pub use device_permission::device_permission_check;
pub(crate) use render::{aggregate_exit_code, render_aggregate_report, render_report};
