use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ops::Deref;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorStatus {
    Ok,
    Warn,
    Fail,
}

impl DoctorStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    fn aggregate(results: &[DoctorCheckResult]) -> Self {
        Self::aggregate_statuses(results.iter().map(|result| result.status))
    }

    fn aggregate_statuses(statuses: impl IntoIterator<Item = DoctorStatus>) -> Self {
        let mut aggregate = Self::Ok;
        for status in statuses {
            if status == Self::Fail {
                return Self::Fail;
            }
            if status == Self::Warn {
                aggregate = Self::Warn;
            }
        }
        aggregate
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PluginDoctorReport {
    pub plugin_id: String,
    pub status: DoctorStatus,
    pub diagnostics: Vec<DoctorCheckResult>,
    pub report: Option<PreservedDoctorReport>,
}

impl PluginDoctorReport {
    pub fn new(
        plugin_id: impl Into<String>,
        diagnostics: Vec<DoctorCheckResult>,
        report: Option<DoctorReport>,
    ) -> Self {
        Self::new_preserved(
            plugin_id,
            diagnostics,
            report.map(PreservedDoctorReport::new),
        )
    }

    pub fn new_preserved(
        plugin_id: impl Into<String>,
        diagnostics: Vec<DoctorCheckResult>,
        report: Option<PreservedDoctorReport>,
    ) -> Self {
        let status = DoctorStatus::aggregate_statuses(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.status)
                .chain(report.iter().map(|report| report.status)),
        );
        Self {
            plugin_id: plugin_id.into(),
            status,
            diagnostics,
            report,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreservedDoctorReport {
    report: DoctorReport,
    raw: Value,
}

impl PreservedDoctorReport {
    pub fn new(report: DoctorReport) -> Self {
        let raw = serde_json::to_value(&report)
            .expect("serializing a doctor report containing JSON values cannot fail");
        Self { report, raw }
    }

    pub fn from_value(raw: Value) -> serde_json::Result<Self> {
        let report = serde_json::from_value(raw.clone())?;
        Ok(Self { report, raw })
    }

    pub fn from_slice(bytes: &[u8]) -> serde_json::Result<Self> {
        Self::from_value(serde_json::from_slice(bytes)?)
    }

    pub fn raw(&self) -> &Value {
        &self.raw
    }
}

impl Deref for PreservedDoctorReport {
    type Target = DoctorReport;

    fn deref(&self) -> &Self::Target {
        &self.report
    }
}

impl Serialize for PreservedDoctorReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PreservedDoctorReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = Value::deserialize(deserializer)?;
        Self::from_value(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DoctorAggregateReport {
    pub status: DoctorStatus,
    pub host: DoctorReport,
    pub plugins: Vec<PluginDoctorReport>,
}

impl DoctorAggregateReport {
    pub fn new(host: DoctorReport, mut plugins: Vec<PluginDoctorReport>) -> Self {
        plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
        let status = DoctorStatus::aggregate_statuses(
            std::iter::once(host.status).chain(plugins.iter().map(|plugin| plugin.status)),
        );
        Self {
            status,
            host,
            plugins,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DoctorReport {
    pub plugin_id: String,
    pub status: DoctorStatus,
    pub checks: Vec<DoctorCheckResult>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl DoctorReport {
    pub fn from_results(plugin_id: impl Into<String>, checks: Vec<DoctorCheckResult>) -> Self {
        let status = DoctorStatus::aggregate(&checks);
        Self {
            plugin_id: plugin_id.into(),
            status,
            checks,
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DoctorCheckResult {
    pub id: String,
    pub status: DoctorStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl DoctorCheckResult {
    pub fn ok(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, DoctorStatus::Ok, message)
    }

    pub fn warn(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, DoctorStatus::Warn, message)
    }

    pub fn fail(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, DoctorStatus::Fail, message)
    }

    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    fn new(id: impl Into<String>, status: DoctorStatus, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status,
            message: message.into(),
            fix: None,
            details: None,
            extensions: BTreeMap::new(),
        }
    }
}
