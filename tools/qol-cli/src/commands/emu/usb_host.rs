use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn validate(path: Option<&Path>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if !path.is_absolute() {
        bail!("--usb-host must be an absolute USB device path");
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::FileTypeExt;
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("failed to inspect USB device {}", path.display()))?;
        if !metadata.file_type().is_char_device() {
            bail!(
                "--usb-host must name a USB character device: {}",
                path.display()
            );
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        bail!("--usb-host is supported only on Linux");
    }
}

pub(crate) fn acquire(path: Option<&Path>, run_id: &str) -> Result<Option<UsbHostLease>> {
    let Some(path) = path else {
        return Ok(None);
    };
    validate(Some(path))?;
    #[cfg(target_os = "linux")]
    {
        linux::spawn(path, run_id).map(|lease| Some(UsbHostLease { inner: Some(lease) }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, run_id);
        bail!("--usb-host is supported only on Linux");
    }
}

pub(crate) fn run_helper(args: &[OsString]) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::run_helper(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        bail!("the USB host lease helper is supported only on Linux");
    }
}

pub(crate) struct UsbHostLease {
    #[cfg(target_os = "linux")]
    inner: Option<linux::LeaseConnection>,
}

impl UsbHostLease {
    pub(crate) fn release(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(mut inner) = self.inner.take() {
            return inner.release();
        }
        Ok(())
    }
}

impl Drop for UsbHostLease {
    fn drop(&mut self) {
        if let Err(error) = self.release() {
            eprintln!("Bluetooth USB host lease cleanup failed: {error:#}");
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use anyhow::{anyhow, bail, Context, Result};
    use qol_host_fixes::elevation;
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::fs::OpenOptions;
    use std::io::{BufRead, BufReader, Write};
    use std::os::fd::AsRawFd;
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    const OWNER_EXIT_GRACE: Duration = Duration::from_secs(30);
    const HELPER_EXIT_GRACE: Duration = Duration::from_secs(10);
    const RESTORE_ATTEMPTS: usize = 6;
    const RESTORE_DELAY: Duration = Duration::from_millis(500);

    pub(super) struct LeaseConnection {
        child: Child,
        stdin: Option<ChildStdin>,
    }

    pub(super) fn spawn(path: &Path, run_id: &str) -> Result<LeaseConnection> {
        let executable = std::env::current_exe().context("failed to locate the qol executable")?;
        let owner_pid = std::process::id();
        let owner_start = process_start_time(owner_pid)
            .with_context(|| format!("failed to identify qol process {owner_pid}"))?;
        let uid = unsafe { libc::getuid() };
        let args = [
            OsString::from("emu"),
            OsString::from("__usb-host-lease"),
            OsString::from("--path"),
            path.as_os_str().to_os_string(),
            OsString::from("--uid"),
            OsString::from(uid.to_string()),
            OsString::from("--owner-pid"),
            OsString::from(owner_pid.to_string()),
            OsString::from("--owner-start"),
            OsString::from(owner_start.to_string()),
            OsString::from("--run-id"),
            OsString::from(run_id),
        ];
        let mut child = elevation::spawn_privileged("qol-bluetooth-usb-lease", &executable, &args)?;
        let stdout = child
            .stdout
            .take()
            .context("privileged USB host lease has no readiness stream")?;
        let mut ready = String::new();
        BufReader::new(stdout)
            .read_line(&mut ready)
            .context("failed to read privileged USB host lease readiness")?;
        if !ready.starts_with("ready\t") {
            let status = child.try_wait().ok().flatten();
            bail!(
                "privileged USB host lease did not become ready{}",
                status
                    .map(|status| format!(" ({status})"))
                    .unwrap_or_default()
            );
        }
        let stdin = child
            .stdin
            .take()
            .context("privileged USB host lease has no control stream")?;
        Ok(LeaseConnection {
            child,
            stdin: Some(stdin),
        })
    }

    impl LeaseConnection {
        pub(super) fn release(&mut self) -> Result<()> {
            let write_result: Result<()> = if let Some(mut stdin) = self.stdin.take() {
                writeln!(stdin, "release")?;
                stdin.flush()?;
                Ok(())
            } else {
                Ok(())
            };
            let wait_result = wait_for_child(&mut self.child, HELPER_EXIT_GRACE);
            match (write_result, wait_result) {
                (Ok(()), Ok(status)) if status.success() => Ok(()),
                (Ok(()), Ok(status)) => bail!("privileged USB host lease exited with {status}"),
                (Err(write), Ok(status)) => {
                    Err(anyhow!("failed to release USB host lease: {write}; helper exited with {status}"))
                }
                (Ok(()), Err(wait)) => Err(wait.context("failed to verify USB host lease cleanup")),
                (Err(write), Err(wait)) => Err(anyhow!(
                    "failed to release USB host lease: {write}; cleanup verification failed: {wait:#}"
                )),
            }
        }
    }

    struct HelperArgs {
        path: PathBuf,
        uid: u32,
        owner_pid: u32,
        owner_start: u64,
        run_id: String,
    }

    pub(super) fn run_helper(args: &[OsString]) -> Result<()> {
        let args = parse_helper_args(args)?;
        let mut lease = LeaseState::acquire(&args)?;
        println!("ready\t{}\t{}", lease.sysfs.display(), args.run_id);
        std::io::stdout()
            .flush()
            .context("failed to publish USB host lease readiness")?;
        let release_requested = wait_for_command(&mut std::io::stdin().lock())?;
        if !release_requested {
            wait_for_owner_exit(args.owner_pid, args.owner_start, OWNER_EXIT_GRACE)?;
        }
        lease.restore()
    }

    fn parse_helper_args(args: &[OsString]) -> Result<HelperArgs> {
        let mut values = BTreeMap::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let key = arg
                .to_str()
                .context("USB host lease helper received a non-UTF-8 option")?;
            let value = iter
                .next()
                .with_context(|| format!("{key} needs a value"))?
                .to_str()
                .map(str::to_string)
                .with_context(|| format!("{key} needs a UTF-8 value"))?;
            if !matches!(
                key,
                "--path" | "--uid" | "--owner-pid" | "--owner-start" | "--run-id"
            ) {
                bail!("unknown USB host lease helper option `{key}`");
            }
            if values.insert(key.to_string(), value).is_some() {
                bail!("duplicate USB host lease helper option `{key}`");
            }
        }
        let path = PathBuf::from(required_value(&values, "--path")?);
        if !path.is_absolute() {
            bail!("USB host lease path must be absolute");
        }
        let uid = parse_value(&values, "--uid")?;
        let owner_pid = parse_value(&values, "--owner-pid")?;
        let owner_start = parse_value(&values, "--owner-start")?;
        let run_id = required_value(&values, "--run-id")?;
        if run_id.is_empty()
            || run_id.len() > 64
            || !run_id.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_uppercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_')
            })
        {
            bail!("USB host lease run id is invalid");
        }
        Ok(HelperArgs {
            path,
            uid,
            owner_pid,
            owner_start,
            run_id,
        })
    }

    fn required_value(values: &BTreeMap<String, String>, key: &str) -> Result<String> {
        values
            .get(key)
            .cloned()
            .with_context(|| format!("missing {key}"))
    }

    fn parse_value<T>(values: &BTreeMap<String, String>, key: &str) -> Result<T>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        required_value(values, key)?
            .parse()
            .map_err(|error| anyhow!("invalid {key}: {error}"))
    }

    fn wait_for_command(reader: &mut impl BufRead) -> Result<bool> {
        let mut line = String::new();
        loop {
            line.clear();
            let count = reader
                .read_line(&mut line)
                .context("failed to read USB host lease control stream")?;
            if count == 0 {
                return Ok(false);
            }
            let command = line.trim();
            if command == "release" {
                return Ok(true);
            }
            if let Some(pid) = command.strip_prefix("qemu ") {
                pid.parse::<u32>()
                    .with_context(|| format!("invalid qemu PID `{pid}`"))?;
                continue;
            }
            bail!("unknown USB host lease control command `{command}`");
        }
    }

    fn wait_for_owner_exit(pid: u32, start: u64, grace: Duration) -> Result<()> {
        let deadline = Instant::now() + grace;
        while process_start_time(pid) == Some(start) {
            if Instant::now() >= deadline {
                bail!("qol owner process {pid} did not exit before USB host lease cleanup");
            }
            thread::sleep(RESTORE_DELAY);
        }
        Ok(())
    }

    fn wait_for_child(child: &mut Child, grace: Duration) -> Result<std::process::ExitStatus> {
        let deadline = Instant::now() + grace;
        loop {
            if let Some(status) = child.try_wait().context("failed to poll USB host lease")? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                bail!(
                    "privileged USB host lease did not exit within {} seconds",
                    grace.as_secs()
                );
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    struct BoundInterface {
        name: String,
        sysfs: PathBuf,
        driver: PathBuf,
    }

    struct LeaseState {
        node: PathBuf,
        sysfs: PathBuf,
        vendor: String,
        product: String,
        original_acl: Vec<u8>,
        interfaces: Vec<BoundInterface>,
        restored: bool,
    }

    impl LeaseState {
        fn acquire(args: &HelperArgs) -> Result<Self> {
            let sysfs = find_sysfs_device(&args.path)?;
            let vendor = read_sysfs_value(&sysfs, "idVendor")?;
            let product = read_sysfs_value(&sysfs, "idProduct")?;
            let interfaces = btusb_interfaces(&sysfs)?;
            if interfaces.is_empty() {
                bail!(
                    "USB host device {} has no bound btusb interface",
                    args.path.display()
                );
            }
            let original_acl = capture_acl(&args.path)?;
            let mut lease = Self {
                node: args.path.clone(),
                sysfs,
                vendor,
                product,
                original_acl,
                interfaces,
                restored: false,
            };
            if let Err(error) = lease.detach_and_grant(args.uid) {
                let cleanup = lease.restore();
                return Err(match cleanup {
                    Ok(()) => anyhow!("USB host lease setup failed: {error:#}"),
                    Err(cleanup) => anyhow!(
                        "USB host lease setup failed: {error:#}; rollback failed: {cleanup:#}"
                    ),
                });
            }
            Ok(lease)
        }

        fn detach_and_grant(&mut self, uid: u32) -> Result<()> {
            for interface in &self.interfaces {
                write_sysfs(&interface.driver.join("unbind"), &interface.name)
                    .with_context(|| format!("failed to unbind {}", interface.name))?;
            }
            reset_device(&self.node)?;
            let current_node = wait_for_device_node(&self.sysfs)?;
            if current_node != self.node {
                bail!(
                    "USB device node changed from {} to {} during reset",
                    self.node.display(),
                    current_node.display()
                );
            }
            grant_acl(&self.node, uid)
        }

        fn restore(&mut self) -> Result<()> {
            if self.restored {
                return Ok(());
            }
            let mut failures = Vec::new();
            if let Err(error) = self.restore_driver() {
                failures.push(format!("driver restore failed: {error:#}"));
            }
            if let Err(error) = self.restore_acl() {
                failures.push(format!("ACL restore failed: {error:#}"));
            }
            if failures.is_empty() {
                self.restored = true;
                Ok(())
            } else {
                Err(anyhow!(failures.join("; ")))
            }
        }

        fn restore_driver(&self) -> Result<()> {
            let current_vendor = read_sysfs_value(&self.sysfs, "idVendor")?;
            let current_product = read_sysfs_value(&self.sysfs, "idProduct")?;
            if current_vendor != self.vendor || current_product != self.product {
                bail!("USB device identity changed while it was leased");
            }
            for interface in &self.interfaces {
                let bound_driver = interface.sysfs_path().join("driver");
                if bound_driver.exists() {
                    continue;
                }
                let mut last_error = None;
                for _ in 0..RESTORE_ATTEMPTS {
                    match write_sysfs(&interface.driver.join("bind"), &interface.name) {
                        Ok(()) if bound_driver.exists() => {
                            last_error = None;
                            break;
                        }
                        Ok(()) => last_error = Some(anyhow!("driver did not reappear")),
                        Err(error) => last_error = Some(error),
                    }
                    thread::sleep(RESTORE_DELAY);
                }
                if let Some(error) = last_error {
                    return Err(error)
                        .with_context(|| format!("failed to rebind {}", interface.name));
                }
            }
            Ok(())
        }

        fn restore_acl(&self) -> Result<()> {
            let node = current_device_node(&self.sysfs).unwrap_or_else(|| self.node.clone());
            let content = acl_for_path(&self.original_acl, &node)?;
            restore_acl(&content)
        }
    }

    impl BoundInterface {
        fn sysfs_path(&self) -> &Path {
            &self.sysfs
        }
    }

    impl Drop for LeaseState {
        fn drop(&mut self) {
            if !self.restored {
                if let Err(error) = self.restore() {
                    eprintln!("USB host lease rollback failed: {error:#}");
                }
            }
        }
    }

    fn find_sysfs_device(node: &Path) -> Result<PathBuf> {
        let (bus, device) = usb_node_numbers(node)?;
        let root = Path::new("/sys/bus/usb/devices");
        for entry in fs::read_dir(root).context("failed to inspect USB sysfs")? {
            let entry = entry.context("failed to read USB sysfs entry")?;
            let path = entry.path();
            if !path.is_dir() || !path.join("busnum").is_file() || !path.join("devnum").is_file() {
                continue;
            }
            let candidate_bus = read_sysfs_value(&path, "busnum")?;
            let candidate_device = read_sysfs_value(&path, "devnum")?;
            if candidate_bus == bus.to_string() && candidate_device == device.to_string() {
                return Ok(path);
            }
        }
        bail!("USB device {} is not present in sysfs", node.display())
    }

    fn usb_node_numbers(node: &Path) -> Result<(u16, u16)> {
        let device = node
            .file_name()
            .and_then(OsStr::to_str)
            .context("USB device node has no numeric device name")?
            .parse::<u16>()
            .context("USB device node number is invalid")?;
        let bus = node
            .parent()
            .and_then(Path::file_name)
            .and_then(OsStr::to_str)
            .context("USB device node has no numeric bus name")?
            .parse::<u16>()
            .context("USB device bus number is invalid")?;
        Ok((bus, device))
    }

    fn btusb_interfaces(sysfs: &Path) -> Result<Vec<BoundInterface>> {
        let prefix = format!(
            "{}:",
            sysfs
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
        );
        let mut interfaces = Vec::new();
        for entry in fs::read_dir(sysfs).context("failed to inspect USB interfaces")? {
            let entry = entry.context("failed to read USB interface")?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            if !name.starts_with(&prefix) {
                continue;
            }
            let driver = path.join("driver");
            let Ok(driver) = driver.canonicalize() else {
                continue;
            };
            if driver.file_name().and_then(OsStr::to_str) != Some("btusb") {
                continue;
            }
            interfaces.push(BoundInterface {
                name: name.to_string(),
                sysfs: path,
                driver,
            });
        }
        interfaces.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(interfaces)
    }

    fn read_sysfs_value(root: &Path, name: &str) -> Result<String> {
        fs::read_to_string(root.join(name))
            .with_context(|| format!("failed to read {}", root.join(name).display()))
            .map(|value| value.trim().to_string())
    }

    fn write_sysfs(path: &Path, value: &str) -> Result<()> {
        fs::write(path, value.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))
    }

    fn reset_device(path: &Path) -> Result<()> {
        const USBDEVFS_RESET: libc::c_ulong = 0x5514;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("failed to open {} for USB reset", path.display()))?;
        let result = unsafe { libc::ioctl(file.as_raw_fd(), USBDEVFS_RESET) };
        if result < 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("failed to reset USB device {}", path.display()));
        }
        Ok(())
    }

    fn capture_acl(path: &Path) -> Result<Vec<u8>> {
        let output = Command::new("getfacl")
            .args(["--absolute-names", "--"])
            .arg(path)
            .output()
            .context("failed to launch getfacl")?;
        if !output.status.success() {
            bail!("getfacl exited with {}", output.status);
        }
        Ok(output.stdout)
    }

    fn grant_acl(path: &Path, uid: u32) -> Result<()> {
        let status = Command::new("setfacl")
            .args(["-m", &format!("u:{uid}:rw-"), "--"])
            .arg(path)
            .status()
            .context("failed to launch setfacl")?;
        if !status.success() {
            bail!("setfacl grant exited with {status}");
        }
        Ok(())
    }

    fn acl_for_path(original: &[u8], path: &Path) -> Result<Vec<u8>> {
        let text = String::from_utf8(original.to_vec()).context("getfacl output was not UTF-8")?;
        let (_, remainder) = text
            .split_once('\n')
            .context("getfacl output has no file header")?;
        Ok(format!("# file: {}\n{remainder}", path.display()).into_bytes())
    }

    fn restore_acl(content: &[u8]) -> Result<()> {
        let mut command = Command::new("setfacl");
        command
            .arg("--restore=-")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .context("failed to launch setfacl restore")?;
        child
            .stdin
            .take()
            .context("setfacl restore has no input stream")?
            .write_all(content)
            .context("failed to send original ACL to setfacl")?;
        let output = child
            .wait_with_output()
            .context("failed to wait for setfacl restore")?;
        if !output.status.success() {
            bail!(
                "setfacl restore exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn current_device_node(sysfs: &Path) -> Option<PathBuf> {
        let bus = read_sysfs_value(sysfs, "busnum")
            .ok()?
            .parse::<u16>()
            .ok()?;
        let device = read_sysfs_value(sysfs, "devnum")
            .ok()?
            .parse::<u16>()
            .ok()?;
        let path = PathBuf::from(format!("/dev/bus/usb/{bus:03}/{device:03}"));
        path.exists().then_some(path)
    }

    fn wait_for_device_node(sysfs: &Path) -> Result<PathBuf> {
        for _ in 0..RESTORE_ATTEMPTS {
            if let Some(path) = current_device_node(sysfs) {
                return Ok(path);
            }
            thread::sleep(RESTORE_DELAY);
        }
        bail!("USB device node did not return after reset")
    }

    fn process_start_time(pid: u32) -> Option<u64> {
        let content = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let (_, remainder) = content.rsplit_once(") ")?;
        remainder.split_whitespace().nth(19)?.parse().ok()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn helper_args_require_a_safe_run_id_and_exact_options() {
            let args = vec![
                OsString::from("--path"),
                OsString::from("/dev/bus/usb/001/007"),
                OsString::from("--uid"),
                OsString::from("1000"),
                OsString::from("--owner-pid"),
                OsString::from("42"),
                OsString::from("--owner-start"),
                OsString::from("99"),
                OsString::from("--run-id"),
                OsString::from("mint-run-1"),
            ];
            let parsed = parse_helper_args(&args).unwrap();
            assert_eq!(parsed.path, PathBuf::from("/dev/bus/usb/001/007"));
            assert_eq!(parsed.uid, 1000);
            assert_eq!(parsed.owner_pid, 42);
            assert_eq!(parsed.owner_start, 99);
            assert_eq!(parsed.run_id, "mint-run-1");
        }

        #[test]
        fn helper_args_reject_duplicate_and_unsafe_values() {
            let duplicate = vec![
                OsString::from("--path"),
                OsString::from("/dev/null"),
                OsString::from("--path"),
                OsString::from("/dev/null"),
            ];
            assert!(parse_helper_args(&duplicate).is_err());

            let unsafe_id = vec![
                OsString::from("--path"),
                OsString::from("/dev/null"),
                OsString::from("--uid"),
                OsString::from("1000"),
                OsString::from("--owner-pid"),
                OsString::from("42"),
                OsString::from("--owner-start"),
                OsString::from("99"),
                OsString::from("--run-id"),
                OsString::from("../run"),
            ];
            assert!(parse_helper_args(&unsafe_id).is_err());
        }

        #[test]
        fn acl_restore_retargets_only_the_file_header() {
            let original = b"# file: /dev/bus/usb/001/007\n# owner: root\n# group: root\nuser::rw-\nuser:1000:rw-\n";
            let rewritten = acl_for_path(original, Path::new("/dev/bus/usb/001/009")).unwrap();
            assert_eq!(
                String::from_utf8(rewritten).unwrap(),
                "# file: /dev/bus/usb/001/009\n# owner: root\n# group: root\nuser::rw-\nuser:1000:rw-\n"
            );
        }

        #[test]
        fn current_process_has_a_start_time_identity() {
            assert!(process_start_time(std::process::id()).is_some());
        }
    }
}
