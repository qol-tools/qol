# Platform test isolation

The tray's default test runtime root is private and thread-local. On macOS it is allocated beneath `/tmp`, because the system temporary directory can leave insufficient room for daemon socket names and dev-generation suffixes within `sockaddr_un`. Explicit test path overrides retain their existing meaning. Production runtime paths are unchanged.

Filesystem watchers may deliver ancestor or setup events before the requested file event. Watch tests consume notifications until the expected path arrives within one fixed deadline. They still require the specific file or directory event and fail on timeout or disconnection.

The Linux policy-lock exec test uses the lock's bounded acquisition operation after releasing the original guard. Kernel references can briefly outlive descriptor closure. A leaked descriptor still fails: reacquisition must complete while the child remains alive, and the child is killed and reaped before an acquisition failure is asserted.

The privileged-process timeout test accepts an observed terminal state once. Re-reading a PID after observing a zombie can race with its reaping; combining a successful existence probe with a later missing `/proc` entry can temporarily report it as alive again. Both root and descendant must be observed gone within the original deadline.

These are test lifecycle corrections. Existing CI failure output and process-containment diagnostics cover the relevant evidence; no new runtime trace target is needed.
