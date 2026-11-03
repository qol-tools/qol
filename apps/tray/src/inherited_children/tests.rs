use std::cell::RefCell;

use super::*;

fn entry(pid: u32, parent: u32) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent,
        exe: "qol-plugin".into(),
    }
}

#[test]
fn only_processes_whose_parent_is_this_tray_are_inherited() {
    let processes = [entry(10, 1), entry(11, 5), entry(12, 5), entry(5, 1)];

    assert_eq!(inherited(&processes, 5), BTreeSet::from([11, 12]));
    assert!(inherited(&processes, 99).is_empty());
}

#[test]
fn children_are_waited_on_until_each_has_exited() {
    let checks = RefCell::new(Vec::new());
    let try_wait = |pid: u32| {
        checks.borrow_mut().push(pid);
        let round = checks.borrow().iter().filter(|&&seen| seen == pid).count();
        match pid {
            11 if round < 3 => Ok(None),
            12 => Err(io::Error::other("not a child of this process")),
            _ => Ok(Some(ExitStatus::default())),
        }
    };

    reap_until_gone(BTreeSet::from([10, 11, 12]), try_wait, Duration::ZERO);

    assert_eq!(*checks.borrow(), vec![10, 11, 12, 11, 11]);
}

#[cfg(unix)]
#[test]
#[allow(clippy::zombie_processes)]
fn an_exited_child_stops_being_a_zombie() {
    let child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    std::mem::forget(child);

    reap_until_gone(
        BTreeSet::from([pid]),
        qol_process::try_wait_pid,
        Duration::from_millis(10),
    );

    assert!(!qol_process::is_pid_zombie(pid));
}
