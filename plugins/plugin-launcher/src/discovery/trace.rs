use super::search::{Fuzziness, SearchMode};

pub(super) struct FilterSample<'a> {
    pub path: &'static str,
    pub query: &'a str,
    pub mode: SearchMode,
    pub fuzziness: Fuzziness,
    pub app_count: usize,
    pub file_count: usize,
    pub candidate_count: usize,
    pub result_count: usize,
    pub elapsed_us: u128,
}

pub(super) fn filter(sample: FilterSample<'_>) {
    qol_runtime::probe!(
        "LAUNCHER_FILTER",
        "path={} mode={} fuzz={} q=\"{}\" q_len={} apps={} files={} candidates={} results={} elapsed_us={}",
        sample.path,
        sample.mode.label(),
        sample.fuzziness.label(),
        quoted(sample.query),
        sample.query.chars().count(),
        sample.app_count,
        sample.file_count,
        sample.candidate_count,
        sample.result_count,
        sample.elapsed_us,
    );
}

fn quoted(value: &str) -> String {
    value
        .chars()
        .take(120)
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' {
                c
            } else if c == ' ' {
                ' '
            } else {
                '_'
            }
        })
        .collect()
}
