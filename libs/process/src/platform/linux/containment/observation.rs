use super::{directory_identity, open_at, open_directory, parse_linux_process_identity};
use super::{LinuxProcessHandle, ProcessTreeGuard};
use crate::{MemberObservation, NodeObservation, Observation, ProcessProvenance, ProcessStat};
use crate::{ProcessTreeObservation, ScopeIdentity};
use std::ffi::CStr;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd};
use std::path::Path;
use std::time::{Duration, Instant};

const BUDGET: Duration = Duration::from_millis(10);
const READ_LIMIT: u64 = 4096;
const MEMBER_LIMIT: usize = 64;
const NODE_LIMIT: usize = 32;
const DEPTH_LIMIT: usize = 8;

#[derive(Debug)]
enum Failure {
    Vanished,
    Unavailable,
    Reused,
    Truncated,
    BudgetExceeded,
}

type Sample<T> = Result<T, Failure>;

struct Collector {
    started: Instant,
    report: ProcessTreeObservation,
}

struct DirectoryStream(*mut libc::DIR);

impl Drop for DirectoryStream {
    fn drop(&mut self) {
        unsafe { libc::closedir(self.0) };
    }
}

impl ProcessTreeGuard {
    pub(crate) fn containment_backend(&self) -> &'static str {
        "linux_cgroup_v2"
    }

    pub(crate) fn membership_observation_supported(&self) -> bool {
        true
    }

    pub(crate) fn observe_residual(&self) -> ProcessTreeObservation {
        let mut collector = Collector {
            started: Instant::now(),
            report: ProcessTreeObservation::unsupported(),
        };
        collector.report.support = "bounded_owned_subtree";
        collector.report.incomplete = false;
        collector.report.timing_perturbed = true;
        collector.report.creator = provenance(Some(self.cgroup.creator_pid as u32));
        collector.report.guardian = provenance(
            self.guardian
                .as_ref()
                .and_then(|guardian| guardian.spawned_pid()),
        );
        collector.report.leader = provenance(None);
        if let Ok(target) = self.target.try_lock() {
            if let Some(target) = target.as_ref() {
                collector.report.leader = provenance(Some(target.root_pid as u32));
                if let Some(root) = target.root.as_ref() {
                    collector.report.leader.generation = Observation::Value(root.generation);
                    collector.report.leader.start_ticks =
                        parse_linux_process_identity(&root.identity)
                            .map(|identity| Observation::Value(identity.start_ticks))
                            .unwrap_or(Observation::Unavailable);
                }
            }
        }
        let scope = identity(&self.cgroup.directory);
        collector.report.scope = collector.record(scope);
        let before = collector.populated(self.cgroup.directory.as_raw_fd());
        collector.report.root_populated_before = collector.record(before);
        collector.visit(&self.cgroup.directory, None, 0);
        let after = collector.populated(self.cgroup.directory.as_raw_fd());
        collector.report.root_populated_after = collector.record(after);
        collector.report.duration = collector.started.elapsed();
        if collector.report.duration >= BUDGET {
            collector.report.budget_exceeded = true;
            collector.report.incomplete = true;
        }
        collector.report
    }
}

fn provenance(pid: Option<u32>) -> ProcessProvenance {
    ProcessProvenance {
        pid: pid
            .map(Observation::Value)
            .unwrap_or(Observation::NotCaptured),
        start_ticks: Observation::NotCaptured,
        generation: Observation::NotCaptured,
    }
}

fn io_failure(error: io::Error) -> Failure {
    match error.kind() {
        io::ErrorKind::NotFound => Failure::Vanished,
        _ if error.raw_os_error() == Some(libc::ESRCH) => Failure::Vanished,
        _ => Failure::Unavailable,
    }
}

fn identity(directory: &OwnedFd) -> Sample<ScopeIdentity> {
    directory_identity(directory)
        .map(|(device, inode)| ScopeIdentity { device, inode })
        .map_err(io_failure)
}

impl Collector {
    fn check_budget(&self) -> Sample<()> {
        if self.started.elapsed() >= BUDGET {
            return Err(Failure::BudgetExceeded);
        }
        Ok(())
    }

    fn record<T>(&mut self, sample: Sample<T>) -> Observation<T> {
        match sample {
            Ok(value) => Observation::Value(value),
            Err(failure) => {
                self.report.incomplete = true;
                match failure {
                    Failure::Vanished => Observation::Vanished,
                    Failure::Unavailable => Observation::Unavailable,
                    Failure::Reused => Observation::Reused,
                    Failure::Truncated => {
                        self.report.truncated = true;
                        Observation::Truncated
                    }
                    Failure::BudgetExceeded => {
                        self.report.budget_exceeded = true;
                        Observation::BudgetExceeded
                    }
                }
            }
        }
    }

    fn read(&self, directory: i32, name: &str) -> Sample<String> {
        self.check_budget()?;
        let descriptor = open_at(directory, name, libc::O_RDONLY).map_err(io_failure)?;
        self.check_budget()?;
        let mut reader = File::from(descriptor).take(READ_LIMIT);
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            self.check_budget()?;
            let count = reader.read(&mut buffer).map_err(io_failure)?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if bytes.len() as u64 == READ_LIMIT {
                return Err(Failure::Truncated);
            }
        }
        String::from_utf8(bytes).map_err(|_| Failure::Unavailable)
    }

    fn populated(&self, directory: i32) -> Sample<bool> {
        self.read(directory, "cgroup.events")?
            .lines()
            .find_map(|line| match line {
                "populated 0" => Some(false),
                "populated 1" => Some(true),
                _ => None,
            })
            .ok_or(Failure::Unavailable)
    }

    fn visit(&mut self, directory: &OwnedFd, parent: Option<usize>, depth: usize) {
        if let Err(failure) = self.check_budget() {
            self.record::<()>(Err(failure));
            return;
        }
        let index = self.report.nodes.len();
        let node_identity = self.record(identity(directory));
        let populated = self.populated(directory.as_raw_fd());
        let populated = self.record(populated);
        self.report.nodes.push(NodeObservation {
            index,
            parent,
            depth,
            identity: node_identity,
            populated,
            membership: Observation::NotCaptured,
        });
        let members = self.members(directory, index);
        self.report.nodes[index].membership = self.record(members);
        let children = self.children(directory, index, depth);
        self.record(children);
    }

    fn members(&mut self, directory: &OwnedFd, node: usize) -> Sample<usize> {
        let content = self.read(directory.as_raw_fd(), "cgroup.procs")?;
        let mut count = 0;
        for line in content.lines() {
            self.check_budget()?;
            if self.report.members.len() == MEMBER_LIMIT {
                return Err(Failure::Truncated);
            }
            let pid = line.parse::<u32>().map_err(|_| Failure::Unavailable)?;
            if pid == 0 || pid > i32::MAX as u32 {
                return Err(Failure::Unavailable);
            }
            let member = self.member(node, pid);
            self.report.members.push(member);
            count += 1;
        }
        Ok(count)
    }

    fn children(&mut self, directory: &OwnedFd, parent: usize, depth: usize) -> Sample<()> {
        self.check_budget()?;
        let descriptor = open_at(
            directory.as_raw_fd(),
            ".",
            libc::O_RDONLY | libc::O_DIRECTORY,
        )
        .map_err(io_failure)?;
        let raw = descriptor.into_raw_fd();
        let stream = unsafe { libc::fdopendir(raw) };
        if stream.is_null() {
            unsafe { libc::close(raw) };
            return Err(Failure::Unavailable);
        }
        let stream = DirectoryStream(stream);
        loop {
            self.check_budget()?;
            unsafe { *libc::__errno_location() = 0 };
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                if unsafe { *libc::__errno_location() } != 0 {
                    return Err(Failure::Unavailable);
                }
                return Ok(());
            }
            let entry = unsafe { &*entry };
            let name = unsafe { CStr::from_ptr(entry.d_name.as_ptr()) };
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            if entry.d_type != libc::DT_DIR && entry.d_type != libc::DT_UNKNOWN {
                continue;
            }
            if depth == DEPTH_LIMIT || self.report.nodes.len() == NODE_LIMIT {
                return Err(Failure::Truncated);
            }
            self.check_budget()?;
            let name = name.to_str().map_err(|_| Failure::Unavailable)?;
            let child = match open_at(
                directory.as_raw_fd(),
                name,
                libc::O_RDONLY | libc::O_DIRECTORY,
            ) {
                Ok(child) => child,
                Err(error) if error.raw_os_error() == Some(libc::ENOTDIR) => continue,
                Err(error) => return Err(io_failure(error)),
            };
            self.visit(&child, Some(parent), depth + 1);
        }
    }

    fn stat(&self, pid: u32) -> Sample<ProcessStat> {
        self.check_budget()?;
        let directory = open_directory(Path::new(&format!("/proc/{pid}"))).map_err(io_failure)?;
        let proc_identity = identity(&directory)?;
        let stat = self.read(directory.as_raw_fd(), "stat")?;
        parse_stat(&stat, proc_identity)
    }

    fn member(&mut self, node: usize, pid: u32) -> MemberObservation {
        let stat = self.stat(pid);
        let handle = self
            .check_budget()
            .and_then(|()| LinuxProcessHandle::open(pid as i32).map_err(io_failure));
        let (identity_check, pidfd_alive) = match (&stat, handle) {
            (Ok(first), Ok(handle)) => {
                let checked = self.check_identity(pid, first, &handle);
                let alive = self
                    .check_budget()
                    .and_then(|()| handle.is_alive().map_err(io_failure));
                (self.record(checked), self.record(alive))
            }
            (_, Err(failure)) => (Observation::NotCaptured, self.record::<bool>(Err(failure))),
            (Err(_), Ok(handle)) => {
                let alive = self
                    .check_budget()
                    .and_then(|()| handle.is_alive().map_err(io_failure));
                (Observation::NotCaptured, self.record(alive))
            }
        };
        MemberObservation {
            node,
            pid,
            stat: self.record(stat),
            identity_check,
            pidfd_alive,
        }
    }

    fn check_identity(
        &self,
        pid: u32,
        first: &ProcessStat,
        handle: &LinuxProcessHandle,
    ) -> Sample<bool> {
        self.check_budget()?;
        let directory = open_directory(Path::new("/proc/self/fdinfo")).map_err(io_failure)?;
        let content = self.read(directory.as_raw_fd(), &handle.fd.as_raw_fd().to_string())?;
        let actual = content
            .lines()
            .find_map(|line| line.strip_prefix("Pid:"))
            .and_then(|value| value.trim().parse::<i32>().ok())
            .ok_or(Failure::Unavailable)?;
        if actual == -1 {
            return Err(Failure::Vanished);
        }
        if actual != pid as i32 {
            return Err(Failure::Reused);
        }
        let second = self.stat(pid)?;
        if first.start_ticks != second.start_ticks || first.proc_identity != second.proc_identity {
            return Err(Failure::Reused);
        }
        Ok(true)
    }
}

fn parse_stat(content: &str, proc_identity: ScopeIdentity) -> Sample<ProcessStat> {
    let (_, fields) = content.rsplit_once(") ").ok_or(Failure::Unavailable)?;
    let mut fields = fields.split_whitespace();
    let state = fields
        .next()
        .and_then(|value| value.chars().next())
        .ok_or(Failure::Unavailable)?;
    let mut number = || {
        fields
            .next()
            .and_then(|value| value.parse::<i32>().ok())
            .ok_or(Failure::Unavailable)
    };
    let ppid = number()?;
    let pgid = number()?;
    let sid = number()?;
    let start_ticks = fields
        .nth(15)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(Failure::Unavailable)?;
    Ok(ProcessStat {
        start_ticks,
        state,
        ppid,
        pgid,
        sid,
        proc_identity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collector() -> Collector {
        let mut report = ProcessTreeObservation::unsupported();
        report.incomplete = false;
        Collector {
            started: Instant::now(),
            report,
        }
    }

    #[test]
    fn bounded_reads_report_truncation_unavailability_and_budget_exhaustion() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("large"),
            vec![b'1'; READ_LIMIT as usize + 1],
        )
        .unwrap();
        std::fs::write(root.path().join("small"), b"populated 1\n").unwrap();
        std::fs::create_dir(root.path().join("unreadable")).unwrap();
        let directory = open_directory(root.path()).unwrap();
        let mut collector = collector();
        let value = collector.read(directory.as_raw_fd(), "large");
        assert!(matches!(
            collector.record(value),
            Observation::Truncated | Observation::BudgetExceeded
        ));
        assert!(collector.report.incomplete);
        collector.started = Instant::now();
        let value = collector.read(directory.as_raw_fd(), "missing");
        assert!(matches!(
            collector.record(value),
            Observation::Vanished | Observation::BudgetExceeded
        ));
        collector.started = Instant::now();
        let value = collector.read(directory.as_raw_fd(), "unreadable");
        assert!(matches!(
            collector.record(value),
            Observation::Unavailable | Observation::BudgetExceeded
        ));
        collector.started = Instant::now() - BUDGET;
        let value = collector.read(directory.as_raw_fd(), "small");
        assert!(matches!(
            collector.record(value),
            Observation::BudgetExceeded
        ));
        assert!(collector.report.budget_exceeded);
    }

    #[test]
    fn stat_projection_uses_start_ticks_and_ignores_opaque_names() {
        for (name, state, ppid, pgid, sid, ticks) in [
            ("opaque", 'S', 1, 12, 12, 123_u64),
            ("a ) tricky ( name", 'Z', 99, 44, 40, u64::MAX),
        ] {
            let fields = format!(
                "{state} {ppid} {pgid} {sid} {} {ticks}",
                ["0"; 15].join(" ")
            );
            let stat = parse_stat(
                &format!("42 ({name}) {fields}"),
                ScopeIdentity {
                    device: 1,
                    inode: 2,
                },
            )
            .unwrap();
            assert_eq!(stat.start_ticks, ticks, "{name}");
            assert_eq!(
                (stat.state, stat.ppid, stat.pgid, stat.sid),
                (state, ppid, pgid, sid)
            );
        }
        assert!(parse_stat(
            "truncated",
            ScopeIdentity {
                device: 1,
                inode: 2
            }
        )
        .is_err());
    }

    #[test]
    fn subtree_observation_never_follows_symlinks_or_exceeds_topology_limits() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let initialize = |path: &Path| {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join("cgroup.events"), "populated 0\n").unwrap();
            std::fs::write(path.join("cgroup.procs"), "").unwrap();
        };
        initialize(root.path());
        initialize(outside.path());
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        let mut deep = root.path().to_path_buf();
        for _ in 0..DEPTH_LIMIT + 2 {
            deep.push("opaque");
            initialize(&deep);
        }
        for index in 0..NODE_LIMIT + 2 {
            initialize(&root.path().join(index.to_string()));
        }
        let directory = open_directory(root.path()).unwrap();
        let mut collector = collector();
        collector.visit(&directory, None, 0);
        assert!(collector.report.nodes.len() <= NODE_LIMIT);
        assert!(collector
            .report
            .nodes
            .iter()
            .all(|node| node.depth <= DEPTH_LIMIT));
        assert!(collector.report.incomplete);
        assert!(collector.report.truncated || collector.report.budget_exceeded);
        let outside = identity(&open_directory(outside.path()).unwrap()).unwrap();
        assert!(collector
            .report
            .nodes
            .iter()
            .all(|node| { node.identity != Observation::Value(outside.clone()) }));
    }
}
