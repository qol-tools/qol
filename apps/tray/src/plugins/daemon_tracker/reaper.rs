use super::ManagedProcess;

pub fn kill_orphan_daemons() {
    super::platform::kill_orphan_daemons();
}

pub fn kill_managed_processes(processes: &[ManagedProcess]) -> usize {
    let roots = super::ManagedRoots::load();
    processes
        .iter()
        .filter(|process| super::platform::kill_managed_process(process, &roots))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn kill_managed_process_rejects_changed_executable() {
        let process = ManagedProcess {
            pid: std::process::id() as i32,
            executable: PathBuf::from("/different/plugin-binary"),
        };

        let roots = super::super::ManagedRoots::load();
        assert!(!super::super::platform::kill_managed_process(
            &process, &roots
        ));
    }
}
