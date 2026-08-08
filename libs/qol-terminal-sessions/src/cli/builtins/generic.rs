use crate::cli::{CliSessionDescriptor, CliSessionStrategy, CliTool};
use crate::SessionFacts;

use super::{generic_tool, project_name};

pub(in crate::cli) struct GenericStrategy {
    tool: CliTool,
}

impl Default for GenericStrategy {
    fn default() -> Self {
        Self {
            tool: generic_tool(),
        }
    }
}

impl CliSessionStrategy for GenericStrategy {
    fn tool(&self) -> &CliTool {
        &self.tool
    }

    fn matches(&self, _session: &SessionFacts) -> bool {
        true
    }

    fn interrupt_key(&self) -> &'static str {
        "ctrl+c"
    }

    fn describe(&self, session: &SessionFacts) -> CliSessionDescriptor {
        let reported = session.reported_cmd.as_deref().map(str::trim);
        let title = session.title.trim();
        let display_name = reported
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| (!title.is_empty()).then(|| title.to_owned()))
            .or_else(|| project_name(&session.cwd));
        CliSessionDescriptor {
            tool: self.tool.clone(),
            display_name,
            external_id: None,
            has_activity: None,
        }
    }
}
