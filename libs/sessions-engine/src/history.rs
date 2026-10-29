use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use qol_terminal_sessions::cli::{CliConversation, CliSessionInterpreter};
use qol_terminal_sessions::closed::{self, ClosedRecord};

use super::bridge::BRIDGE_HEADER;
use super::fork::{brief_in_prompt, ForkStore, FORK_HEADER};

const PROMPT_TITLE_CHARS: usize = 80;
const RESERVED_PROMPT_PREFIX: &str = "[qol ";
const HARNESS_BLOCK_PREFIX: char = '<';

pub(super) struct ClosedEntry {
    pub(super) record: ClosedRecord,
    pub(super) pr_urls: Vec<String>,
    pub(super) last_prompt: Option<String>,
}

pub(super) struct History<'a> {
    pub(super) interpreter: &'a CliSessionInterpreter,
    pub(super) closed_dir: &'a Path,
    pub(super) forks: &'a ForkStore,
    pub(super) switchboard_dir: &'a Path,
}

impl History<'_> {
    pub(super) fn entries(&self, now: u64) -> std::io::Result<Vec<ClosedEntry>> {
        let mut entries = closed::records(self.closed_dir)?
            .into_iter()
            .filter(|record| !closed::expired(record, now))
            .map(|record| ClosedEntry {
                record,
                pr_urls: Vec::new(),
                last_prompt: None,
            })
            .collect::<Vec<_>>();
        for conversation in self.conversations(now) {
            if let Some(entry) = entries
                .iter_mut()
                .find(|entry| entry.record.external_id == conversation.external_id)
            {
                entry.last_prompt = last_prompt(&conversation);
                entry.pr_urls = conversation.pr_urls;
                continue;
            }
            if let Some(record) = self.record(&conversation) {
                entries.push(ClosedEntry {
                    record,
                    last_prompt: last_prompt(&conversation),
                    pr_urls: conversation.pr_urls,
                });
            }
        }
        entries.sort_by(|left, right| {
            right
                .record
                .closed_at
                .cmp(&left.record.closed_at)
                .then_with(|| left.record.external_id.cmp(&right.record.external_id))
        });
        Ok(entries)
    }

    pub(super) fn find(&self, external_id: &str, now: u64) -> Option<ClosedRecord> {
        closed::find(self.closed_dir, external_id)
            .filter(|record| !closed::expired(record, now))
            .or_else(|| {
                self.conversations(now)
                    .into_iter()
                    .find(|conversation| conversation.external_id == external_id)
                    .and_then(|conversation| self.record(&conversation))
            })
    }

    pub(super) fn search(
        &self,
        now: u64,
        external_ids: &HashSet<String>,
        terms: &[String],
    ) -> HashMap<String, String> {
        self.interpreter
            .search_conversations(self.since(now), external_ids, terms)
            .into_iter()
            .map(|found| (found.external_id, found.excerpt))
            .collect()
    }

    fn since(&self, now: u64) -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_secs(now.saturating_sub(closed::RETENTION_SECS))
    }

    fn conversations(&self, now: u64) -> Vec<CliConversation> {
        self.interpreter.closed_conversations(self.since(now))
    }

    fn record(&self, conversation: &CliConversation) -> Option<ClosedRecord> {
        if same_dir(Path::new(&conversation.cwd), self.switchboard_dir) {
            return None;
        }
        let prompt = conversation.first_prompt.as_deref().unwrap_or_default();
        let fork = prompt
            .starts_with(FORK_HEADER)
            .then(|| brief_in_prompt(prompt))
            .flatten()
            .and_then(|brief| self.forks.find_brief(brief));
        let settings = &conversation.settings;
        Some(ClosedRecord {
            external_id: conversation.external_id.clone(),
            tool: conversation.tool.to_string(),
            cwd: conversation.cwd.clone(),
            title: conversation
                .title
                .clone()
                .or_else(|| fork.as_ref().map(|fork| fork.title.clone()))
                .or_else(|| prompt_title(prompt)),
            key: fork.map(|fork| fork.key),
            permission_mode: settings.permission_mode.clone(),
            model: settings.model.clone(),
            effort: settings.effort.clone(),
            claude_config_dir: settings.claude_config_dir.clone(),
            lane: prompt.lines().take(2).any(|line| line == BRIDGE_HEADER),
            closed_at: conversation.last_active,
        })
    }
}

fn same_dir(left: &Path, right: &Path) -> bool {
    left == right
        || matches!(
            (left.canonicalize(), right.canonicalize()),
            (Ok(left), Ok(right)) if left == right
        )
}

fn last_prompt(conversation: &CliConversation) -> Option<String> {
    conversation
        .recent_prompts
        .iter()
        .filter(|prompt| !prompt.starts_with(HARNESS_BLOCK_PREFIX))
        .find_map(|prompt| prompt_title(prompt))
}

fn prompt_title(prompt: &str) -> Option<String> {
    if prompt.starts_with(RESERVED_PROMPT_PREFIX) {
        return None;
    }
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    Some(line.chars().take(PROMPT_TITLE_CHARS).collect())
}
