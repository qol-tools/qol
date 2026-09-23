use super::*;

pub struct CursorSession {
    display: *mut xlib::Display,
    root: xlib::Window,
    base: BaseCursor,
    preferred_source_size: u32,
    active_cursor: Option<xlib::Cursor>,
    applied_cursor: Option<CursorImage>,
    current_scale: f32,
    grow_cursor: Option<CursorImage>,
    xfixes_event_base: Option<i32>,
    catalog: ShapeCatalog,
    tree: Vec<xlib::Window>,
}

unsafe extern "C" fn log_x_error(
    _display: *mut xlib::Display,
    event: *mut xlib::XErrorEvent,
) -> libc::c_int {
    let (code, request) = unsafe { ((*event).error_code, (*event).request_code) };
    eprintln!("[shake-to-grow] ignored X error code={code} request={request}");
    0
}

impl CursorSession {
    pub fn open(scale_factor: u32) -> Result<Self> {
        unsafe { xlib::XSetErrorHandler(Some(log_x_error)) };
        let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
        ensure!(!display.is_null(), "failed to open X11 display");
        let root = unsafe { xlib::XDefaultRootWindow(display) };

        let Some(base) = load_base_cursor(display, scale_factor) else {
            unsafe { xlib::XCloseDisplay(display) };
            ensure!(false, "failed to load base cursor pixels");
            unreachable!();
        };

        Ok(Self {
            display,
            root,
            preferred_source_size: preferred_source_size(base.default_size, scale_factor),
            base,
            active_cursor: None,
            applied_cursor: None,
            current_scale: 1.0,
            grow_cursor: None,
            xfixes_event_base: subscribe_cursor_notifications(display, root),
            catalog: ShapeCatalog::new(),
            tree: Vec::new(),
        })
    }

    pub fn set_scale(&mut self, scale: f32) -> bool {
        self.current_scale = scale;
        if scale <= 1.0 + f32::EPSILON {
            self.restore();
            return true;
        }
        if self.active_cursor.is_none() {
            self.capture_live_cursors();
            self.tree = collect_tree(self.display, self.root);
            crate::cursor::journal::journal_scaled(self.root, &window_ids(&self.tree));
        }
        let scaled = self.grow_cursor.as_ref().and_then(|grow_cursor| {
            scale_cursor_for_display(self.display, self.root, grow_cursor, scale)
        });
        let cursor = if let Some(scaled) = scaled.as_ref() {
            make_cursor_from_frames(self.display, &scaled.frames)
        } else {
            make_cursor_at_scale(self.display, self.root, &self.base, scale)
        };
        let Some(cursor) = cursor else {
            return false;
        };
        self.apply_cursor(cursor);
        self.flush();
        self.applied_cursor = scaled
            .map(|scaled| scaled.applied)
            .or_else(|| load_live_cursor_image(self.display, self.base.default_size));
        if let Some(old_cursor) = self.active_cursor.replace(cursor) {
            unsafe { xlib::XFreeCursor(self.display, old_cursor) };
        }
        true
    }

    pub fn live_cursor_hidden(&mut self) -> bool {
        load_live_cursor_image(self.display, self.base.default_size)
            .is_some_and(|image| is_empty_cursor(&image))
    }

    pub fn refresh(&mut self) -> bool {
        let notified = self.take_cursor_notification();
        if self.active_cursor.is_none() {
            return false;
        }
        if self.current_scale <= 1.0 + f32::EPSILON {
            return false;
        }
        if !notified {
            return false;
        }
        if !live_refresh_enabled() {
            return false;
        }
        let Some(sample) = load_live_cursor_image(self.display, self.base.default_size) else {
            return false;
        };
        if is_empty_cursor(&sample) {
            return false;
        }
        if self
            .applied_cursor
            .as_ref()
            .is_some_and(|applied| same_cursor_image(applied, &sample))
        {
            return false;
        }
        if is_our_enlarged_cursor(
            self.grow_cursor.as_ref(),
            self.applied_cursor.as_ref(),
            &sample,
        ) {
            return false;
        }
        if self
            .grow_cursor
            .as_ref()
            .is_some_and(|grow| same_cursor_image(grow, &sample))
        {
            return self.reapply_active_cursor();
        }
        let sample = with_best_source(
            self.display,
            &self.base,
            &mut self.catalog,
            sample,
            self.preferred_source_size,
        );
        if self.same_grow_source(&sample) {
            self.grow_cursor = Some(sample);
            return self.reapply_active_cursor();
        }
        log_cursor_image("live refresh adopt", &sample);
        self.grow_cursor = Some(sample);
        self.apply_grow_cursor()
    }

    pub fn restore(&mut self) {
        if self.active_cursor.is_none() {
            return;
        }
        restore_root_cursor(self.display, self.root, &self.base);
        self.clear_tree_cursors();
        self.flush();
        if let Some(cursor) = self.active_cursor.take() {
            unsafe { xlib::XFreeCursor(self.display, cursor) };
        }
        self.current_scale = 1.0;
        self.applied_cursor = None;
        self.grow_cursor = None;
        self.tree = Vec::new();
        crate::cursor::journal::clear_journal();
    }

    fn flush(&self) {
        sync(self.display);
    }

    fn apply_cursor(&self, cursor: xlib::Cursor) {
        for window in &self.tree {
            unsafe { xlib::XDefineCursor(self.display, *window, cursor) };
        }
    }

    fn clear_tree_cursors(&self) {
        for window in &self.tree {
            if *window == self.root {
                continue;
            }
            unsafe { xlib::XUndefineCursor(self.display, *window) };
        }
    }

    fn capture_live_cursors(&mut self) {
        let Some(live_cursor) = load_live_cursor_image(self.display, self.base.default_size) else {
            eprintln!("[shake-to-grow] failed to capture live cursor at grow-start");
            return;
        };
        if is_empty_cursor(&live_cursor) {
            eprintln!(
                "[shake-to-grow] live cursor is hidden at grow-start, growing the base cursor (guard missed)"
            );
            return;
        }
        let live_cursor = with_best_source(
            self.display,
            &self.base,
            &mut self.catalog,
            live_cursor,
            self.preferred_source_size,
        );
        log_cursor_image("captured live cursor", &live_cursor);
        self.grow_cursor = Some(live_cursor);
    }

    fn apply_grow_cursor(&mut self) -> bool {
        let scaled = self.grow_cursor.as_ref().and_then(|image| {
            scale_cursor_for_display(self.display, self.root, image, self.current_scale)
        });
        let next_cursor = scaled
            .as_ref()
            .and_then(|scaled| make_cursor_from_frames(self.display, &scaled.frames));
        let Some(next_cursor) = next_cursor else {
            eprintln!("[shake-to-grow] live refresh failed to build scaled cursor");
            return false;
        };
        self.apply_cursor(next_cursor);
        self.flush();
        self.applied_cursor = scaled.map(|scaled| scaled.applied);
        if let Some(old_cursor) = self.active_cursor.replace(next_cursor) {
            unsafe { xlib::XFreeCursor(self.display, old_cursor) };
        }
        true
    }

    fn same_grow_source(&self, sample: &CursorImage) -> bool {
        let Some(new_source) = sample.source.first() else {
            return false;
        };
        let Some(grow_cursor) = self.grow_cursor.as_ref() else {
            return false;
        };
        let Some(current_source) = grow_cursor.source.first() else {
            return false;
        };
        sample.source.len() == grow_cursor.source.len()
            && new_source.width == current_source.width
            && new_source.height == current_source.height
            && new_source.xhot == current_source.xhot
            && new_source.yhot == current_source.yhot
            && new_source.pixels == current_source.pixels
    }

    fn reapply_active_cursor(&mut self) -> bool {
        let Some(cursor) = self.active_cursor else {
            return false;
        };
        self.apply_cursor(cursor);
        self.flush();
        true
    }

    fn take_cursor_notification(&mut self) -> bool {
        let Some(event_base) = self.xfixes_event_base else {
            return false;
        };
        let mut pending = false;
        while unsafe { xlib::XPending(self.display) } > 0 {
            let mut event = std::mem::MaybeUninit::<xlib::XEvent>::uninit();
            unsafe { xlib::XNextEvent(self.display, event.as_mut_ptr()) };
            let event = unsafe { event.assume_init() };
            if event.get_type() != event_base + XFIXES_CURSOR_NOTIFY {
                continue;
            }
            let notify = unsafe {
                *(&event as *const xlib::XEvent as *const xfixes::XFixesCursorNotifyEvent)
            };
            if notify.subtype != XFIXES_DISPLAY_CURSOR_NOTIFY {
                continue;
            }
            pending = true;
        }
        pending
    }
}

impl Drop for CursorSession {
    fn drop(&mut self) {
        self.restore();
        unsafe { xlib::XCloseDisplay(self.display) };
    }
}

fn window_ids(windows: &[xlib::Window]) -> Vec<u64> {
    windows.to_vec()
}

pub(crate) fn recover_scale(root: u64, windows: &[u64]) {
    unsafe { xlib::XSetErrorHandler(Some(log_x_error)) };
    let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
    if display.is_null() {
        return;
    }
    for window in windows {
        if *window == root {
            continue;
        }
        unsafe { xlib::XUndefineCursor(display, *window as xlib::Window) };
    }
    let themed = unsafe { xcursor::XcursorLibraryLoadCursor(display, c"left_ptr".as_ptr()) };
    if themed != 0 {
        unsafe { xlib::XDefineCursor(display, root as xlib::Window, themed) };
        unsafe { xlib::XFreeCursor(display, themed) };
    }
    sync(display);
    unsafe { xlib::XCloseDisplay(display) };
    eprintln!("[os-themes] recovered cursors left scaled after an abnormal exit");
}

pub(super) fn collect_tree(display: *mut xlib::Display, root: xlib::Window) -> Vec<xlib::Window> {
    let mut stack = vec![root];
    let mut tree = Vec::new();
    while let Some(window) = stack.pop() {
        tree.push(window);
        for child in window_children(display, window) {
            stack.push(child);
        }
    }
    tree
}

pub(super) fn restore_root_cursor(
    display: *mut xlib::Display,
    root: xlib::Window,
    base: &BaseCursor,
) {
    let themed = unsafe { xcursor::XcursorLibraryLoadCursor(display, c"left_ptr".as_ptr()) };
    let cursor = if themed != 0 {
        themed
    } else {
        let Some(fallback) = make_cursor_at_scale(display, root, base, 1.0) else {
            return;
        };
        fallback
    };
    unsafe { xlib::XDefineCursor(display, root, cursor) };
    unsafe { xlib::XFreeCursor(display, cursor) };
}

pub(super) fn sync(display: *mut xlib::Display) {
    unsafe { xlib::XSync(display, xlib::False) };
}

pub(super) fn live_refresh_enabled() -> bool {
    std::env::var_os("QOL_OS_THEMES_DISABLE_LIVE_REFRESH").is_none()
}

pub(super) fn window_children(
    display: *mut xlib::Display,
    window: xlib::Window,
) -> Vec<xlib::Window> {
    let mut root = 0;
    let mut parent = 0;
    let mut children: *mut xlib::Window = ptr::null_mut();
    let mut child_count = 0;
    let status = unsafe {
        xlib::XQueryTree(
            display,
            window,
            &mut root,
            &mut parent,
            &mut children,
            &mut child_count,
        )
    };
    if status == 0 || children.is_null() {
        return Vec::new();
    }
    let windows = unsafe { std::slice::from_raw_parts(children, child_count as usize).to_vec() };
    unsafe { xlib::XFree(children as *mut _) };
    windows
}

pub(super) fn subscribe_cursor_notifications(
    display: *mut xlib::Display,
    root: xlib::Window,
) -> Option<i32> {
    let mut event_base = 0;
    let mut error_base = 0;
    let status = unsafe { xfixes::XFixesQueryExtension(display, &mut event_base, &mut error_base) };
    if status == 0 {
        eprintln!("[shake-to-grow] live refresh cursor-notify unavailable");
        return None;
    }
    unsafe { xfixes_select_cursor_input_raw(display, root, XFIXES_DISPLAY_CURSOR_NOTIFY_MASK) };
    sync(display);
    eprintln!("[shake-to-grow] live refresh cursor-notify subscribed event_base={event_base}");
    Some(event_base)
}
