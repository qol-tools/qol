# Settings host shutdown during tests

The settings host shares a resident singleton socket across plugin settings and toasts. `PluginManager::shutdown` intentionally stops that host during tray shutdown. Unit tests exercising manager shutdown previously reached the resident socket whenever they omitted a test path override.

The September 5 investigation found test PID 3301938 issuing a successful stop at 13:32 CEST after settings host PID 3291873 started at 13:31. A second captured host exited successfully during another test process’s stop request, without a panic. This establishes an external shutdown cause for disappearing Settings windows during development checks; it does not rule out unrelated crashes.

`paths::runtime_dir` now uses a private temporary root per unit-test thread by default. Explicit scoped and subprocess path overrides retain precedence. Non-test applications keep their existing runtime location. Temporary roots are removed when their test thread exits.

Regression checks cover the fallback path, thread separation, override restoration, and a real socket exchange proving that plugin-manager shutdown sends its kill to an isolated fake settings host.

Trace decision: enrich `SURFACE_ACTIVATION` with `phase=stop-request test_process=...` at the sender and `phase=host outcome=stop_requested` at the receiver. These distinguish an intentional shutdown from an unexplained disappearance without logging user settings or credentials.
