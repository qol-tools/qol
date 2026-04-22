#[derive(Debug, Clone, Copy)]
pub(crate) enum AxEvent {
    ApplicationActivated,
    FocusedWindowChanged,
    MainWindowChanged,
    WindowCreated,
    WindowDestroyed,
    ApplicationHidden { pid: i32 },
    ApplicationShown { pid: i32 },
    WindowMiniaturized { pid: i32 },
    WindowDeminiaturized { pid: i32 },
}
