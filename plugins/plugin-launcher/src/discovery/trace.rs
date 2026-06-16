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
    let (fuzzy_calls, query_lower, name_lower) = counters();
    qol_runtime::probe!(
        "LAUNCHER_FILTER",
        "path={} mode={} fuzz={} q=\"{}\" q_len={} apps={} files={} candidates={} results={} fuzzy_calls={} query_lower={} name_lower={} elapsed_us={}",
        sample.path,
        sample.mode.label(),
        sample.fuzziness.label(),
        quoted(sample.query),
        sample.query.chars().count(),
        sample.app_count,
        sample.file_count,
        sample.candidate_count,
        sample.result_count,
        fuzzy_calls,
        query_lower,
        name_lower,
        sample.elapsed_us,
    );
}

thread_local! {
    static FUZZY_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static QUERY_LOWER: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static NAME_LOWER: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn reset_counters() {
    FUZZY_CALLS.set(0);
    QUERY_LOWER.set(0);
    NAME_LOWER.set(0);
}

pub(super) fn count_fuzzy_call() {
    FUZZY_CALLS.set(FUZZY_CALLS.get() + 1);
}

pub(super) fn count_query_lower() {
    QUERY_LOWER.set(QUERY_LOWER.get() + 1);
}

pub(super) fn count_name_lower() {
    NAME_LOWER.set(NAME_LOWER.get() + 1);
}

fn counters() -> (usize, usize, usize) {
    (FUZZY_CALLS.get(), QUERY_LOWER.get(), NAME_LOWER.get())
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
