use std::collections::BTreeMap;

use serde_json::{json, Value};

pub const SNAP_THRESHOLD_PX: f32 = 8.0;
pub const NUDGE_STEP: i32 = 10;
pub const NUDGE_STEP_FINE: i32 = 1;
pub const SCREEN_MIN: i32 = -32768;
pub const SCREEN_MAX: i32 = 32767;

pub fn nudge_step(fine: bool) -> i32 {
    if fine {
        NUDGE_STEP_FINE
    } else {
        NUDGE_STEP
    }
}

pub fn snap_threshold(scale: f32) -> i32 {
    let threshold = (SNAP_THRESHOLD_PX / scale).round();
    if !threshold.is_finite() || threshold < 1.0 {
        return 1;
    }
    threshold as i32
}

fn text(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_string)
}

fn int(row: &Value, key: &str) -> Option<i32> {
    i32::try_from(row.get(key).and_then(Value::as_i64)?).ok()
}

fn uint(row: &Value, key: &str) -> Option<i32> {
    i32::try_from(row.get(key).and_then(Value::as_u64)?).ok()
}

fn flag(row: &Value, key: &str) -> bool {
    row.get(key).and_then(Value::as_bool).unwrap_or(false)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    pub id: String,
    pub connector: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub refresh_hz: u32,
    pub primary: bool,
}

impl Display {
    pub fn from_row(row: &Value) -> Option<Self> {
        Some(Self {
            id: text(row, "id")?,
            connector: text(row, "connector").unwrap_or_else(|| "display".to_string()),
            x: int(row, "x")?,
            y: int(row, "y")?,
            width: uint(row, "width")?,
            height: uint(row, "height")?,
            refresh_hz: row.get("refresh_hz").and_then(Value::as_u64).unwrap_or(0) as u32,
            primary: flag(row, "primary"),
        })
    }

    pub fn resolution(&self) -> String {
        resolution_label(self.width, self.height, self.refresh_hz)
    }
}

fn resolution_label(width: i32, height: i32, refresh_hz: u32) -> String {
    if refresh_hz == 0 {
        format!("{width}x{height}")
    } else {
        format!("{width}x{height}@{refresh_hz}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height)
    }

    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeOption {
    pub display_id: String,
    pub token: u64,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub label: String,
    pub current: bool,
    pub writable: bool,
    pub selectable: bool,
}

impl ModeOption {
    pub fn from_row(row: &Value) -> Option<Self> {
        let width = u32::try_from(row.get("width").and_then(Value::as_u64)?).ok()?;
        let height = u32::try_from(row.get("height").and_then(Value::as_u64)?).ok()?;
        let refresh_hz = row.get("refresh_hz").and_then(Value::as_u64).unwrap_or(0) as u32;
        let label = text(row, "label").unwrap_or_else(|| {
            if refresh_hz == 0 {
                format!("{width}x{height}")
            } else {
                format!("{width}x{height}@{refresh_hz}")
            }
        });
        text(row, "id")?;
        Some(Self {
            display_id: text(row, "display_id")?,
            token: row.get("token").and_then(Value::as_u64)?,
            width,
            height,
            refresh_hz,
            label,
            current: flag(row, "current"),
            writable: flag(row, "writable"),
            selectable: flag(row, "selectable"),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub min_x: i32,
    pub min_y: i32,
}

impl Fit {
    pub fn to_client_x(self, x: i32) -> f32 {
        (self.offset_x + (x - self.min_x) as f32 * self.scale).round()
    }

    pub fn to_client_y(self, y: i32) -> f32 {
        (self.offset_y + (y - self.min_y) as f32 * self.scale).round()
    }

    pub fn to_client_width(self, width: i32) -> f32 {
        (width as f32 * self.scale).round()
    }

    pub fn to_client_height(self, height: i32) -> f32 {
        (height as f32 * self.scale).round()
    }

    #[cfg(test)]
    pub fn to_client_delta_x(self, dx: f32) -> f32 {
        (dx * self.scale).round()
    }

    pub fn layout_delta_x(&self, dx: f32) -> i32 {
        (dx / self.scale).round() as i32
    }

    pub fn layout_delta_y(&self, dy: f32) -> i32 {
        (dy / self.scale).round() as i32
    }
}

pub fn bounds(displays: &[Display]) -> Option<Rect> {
    let first = displays.first()?;
    let mut min_x = first.x;
    let mut min_y = first.y;
    let mut max_x = first.x.saturating_add(first.width);
    let mut max_y = first.y.saturating_add(first.height);
    for display in displays.iter().skip(1) {
        min_x = min_x.min(display.x);
        min_y = min_y.min(display.y);
        max_x = max_x.max(display.x.saturating_add(display.width));
        max_y = max_y.max(display.y.saturating_add(display.height));
    }
    Some(Rect {
        x: min_x,
        y: min_y,
        width: max_x.saturating_sub(min_x).max(1),
        height: max_y.saturating_sub(min_y).max(1),
    })
}

pub fn fit(displays: &[Display], width: f32, height: f32, pad: f32) -> Option<Fit> {
    let union = bounds(displays)?;
    let available_width = width - 2.0 * pad;
    let available_height = height - 2.0 * pad;
    if available_width <= 0.0 || available_height <= 0.0 {
        return None;
    }
    let scale = (available_width / union.width as f32)
        .min(available_height / union.height as f32)
        .min(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    Some(Fit {
        scale,
        offset_x: (width - union.width as f32 * scale) / 2.0,
        offset_y: (height - union.height as f32 * scale) / 2.0,
        min_x: union.x,
        min_y: union.y,
    })
}

fn edge_targets(rect: &Rect) -> [i32; 3] {
    [rect.x, rect.right(), rect.x.saturating_add(rect.width / 2)]
}

fn snap_axis(origin: i32, size: i32, targets: impl Iterator<Item = i32>, threshold: i32) -> i32 {
    let moving = [
        origin,
        origin.saturating_add(size),
        origin.saturating_add(size / 2),
    ];
    let mut best: Option<i32> = None;
    for target in targets {
        for edge in moving {
            let offset = target.saturating_sub(edge);
            if offset.abs() > threshold {
                continue;
            }
            if best.is_none_or(|current| offset.abs() < current.abs()) {
                best = Some(offset);
            }
        }
    }
    match best {
        Some(offset) => origin.saturating_add(offset),
        None => origin,
    }
}

pub fn snap_rect(moving: Rect, others: &[Rect], threshold: i32) -> Rect {
    Rect {
        x: snap_axis(
            moving.x,
            moving.width,
            others.iter().flat_map(edge_targets),
            threshold,
        ),
        y: snap_axis(
            moving.y,
            moving.height,
            others.iter().flat_map(edge_targets),
            threshold,
        ),
        width: moving.width,
        height: moving.height,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub id: String,
    pub x: i32,
    pub y: i32,
}

fn normalize_placements(placements: &[Placement]) -> Vec<Placement> {
    let dx = placements
        .iter()
        .map(|placement| placement.x)
        .min()
        .unwrap_or(0)
        .min(0);
    let dy = placements
        .iter()
        .map(|placement| placement.y)
        .min()
        .unwrap_or(0)
        .min(0);
    placements
        .iter()
        .map(|placement| Placement {
            id: placement.id.clone(),
            x: placement.x.saturating_sub(dx),
            y: placement.y.saturating_sub(dy),
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayLayoutIntent {
    Arrange {
        placements: Vec<Placement>,
        primary: String,
    },
    SetMode {
        display_id: String,
        token: u64,
        width: u32,
        height: u32,
        refresh_hz: u32,
    },
}

impl DisplayLayoutIntent {
    pub fn action(&self) -> &'static str {
        match self {
            DisplayLayoutIntent::Arrange { .. } => "arrange",
            DisplayLayoutIntent::SetMode { .. } => "set_mode",
        }
    }

    pub fn input(&self) -> Value {
        match self {
            DisplayLayoutIntent::Arrange {
                placements,
                primary,
            } => {
                let placements: Vec<Value> = placements
                    .iter()
                    .map(
                        |placement| json!({"id": placement.id, "x": placement.x, "y": placement.y}),
                    )
                    .collect();
                json!({"placements": placements, "primary": primary})
            }
            DisplayLayoutIntent::SetMode {
                display_id,
                token,
                width,
                height,
                refresh_hz,
            } => {
                let refresh = if *refresh_hz == 0 {
                    Value::Null
                } else {
                    json!(refresh_hz)
                };
                json!({
                    "id": display_id,
                    "token": token,
                    "width": width,
                    "height": height,
                    "refresh": refresh,
                })
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayLayoutBindings {
    pub query: String,
    pub active_query: Option<String>,
    pub action: String,
    pub active_action: Option<String>,
}

impl DisplayLayoutBindings {
    pub fn new(
        query: impl Into<String>,
        active_query: Option<String>,
        action: impl Into<String>,
        active_action: Option<String>,
    ) -> Self {
        Self {
            query: query.into(),
            active_query,
            action: action.into(),
            active_action,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Drag,
    Nudge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedEdit {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub origin: (i32, i32),
    pub kind: EditKind,
    pub active: bool,
    pub moved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedMode {
    pub display_id: String,
    pub token: u64,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

impl StagedMode {
    pub fn label(&self) -> String {
        if self.refresh_hz == 0 {
            format!("{}x{}", self.width, self.height)
        } else {
            format!("{}x{}@{}", self.width, self.height, self.refresh_hz)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Issue {
    DuplicateDisplay(String),
    PrimaryCount(usize),
    OutOfRange(String),
    Overlap(String, String),
}

#[derive(Clone, Debug)]
pub struct DisplayLayoutState {
    bindings: DisplayLayoutBindings,
    displays: Vec<Display>,
    modes: Vec<ModeOption>,
    selected: Option<String>,
    edit: Option<StagedEdit>,
    staged_modes: BTreeMap<String, StagedMode>,
    primary: Option<String>,
    viewport: (f32, f32),
    pad: f32,
    pending: bool,
    error: Option<String>,
    editing: bool,
}

impl DisplayLayoutState {
    pub fn new(bindings: DisplayLayoutBindings) -> Self {
        Self {
            bindings,
            displays: Vec::new(),
            modes: Vec::new(),
            selected: None,
            edit: None,
            staged_modes: BTreeMap::new(),
            primary: None,
            viewport: (0.0, 0.0),
            pad: 0.0,
            pending: false,
            error: None,
            editing: false,
        }
    }

    pub fn bindings(&self) -> &DisplayLayoutBindings {
        &self.bindings
    }

    pub fn set_viewport(&mut self, width: f32, height: f32, pad: f32) {
        self.viewport = (width, height);
        self.pad = pad;
    }

    pub fn fit(&self) -> Option<Fit> {
        fit(&self.displays, self.viewport.0, self.viewport.1, self.pad)
    }

    pub fn displays(&self) -> &[Display] {
        &self.displays
    }

    pub fn selected(&self) -> Option<&Display> {
        let id = self.selected.as_deref()?;
        self.displays.iter().find(|display| display.id == id)
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    #[cfg(test)]
    pub fn edit(&self) -> Option<&StagedEdit> {
        self.edit.as_ref()
    }

    pub fn editing(&self) -> bool {
        self.editing
    }

    pub fn set_editing(&mut self, editing: bool) {
        self.editing = editing;
    }

    pub fn staged_mode(&self, display_id: &str) -> Option<&StagedMode> {
        self.staged_modes.get(display_id)
    }

    pub fn is_primary(&self, display: &Display) -> bool {
        match self.primary.as_deref() {
            Some(id) => id == display.id,
            None => display.primary,
        }
    }

    pub fn primary_id(&self) -> Option<&str> {
        if let Some(id) = self.primary.as_deref() {
            return Some(id);
        }
        self.displays
            .iter()
            .find(|display| display.primary)
            .map(|display| display.id.as_str())
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    pub fn set_pending(&mut self, pending: bool) {
        self.pending = pending;
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    pub fn load_layout(&mut self, payload: &Value) {
        let rows = payload.as_array().cloned().unwrap_or_default();
        let displays: Vec<Display> = rows.iter().filter_map(Display::from_row).collect();
        self.install_layout(displays);
        self.error = None;
    }

    fn install_layout(&mut self, displays: Vec<Display>) {
        self.displays = displays;
        let keep_edit = self
            .edit
            .as_ref()
            .is_some_and(|edit| !edit.active && self.displays.iter().any(|d| d.id == edit.id));
        if !keep_edit {
            self.edit = None;
        }
        let ids: Vec<String> = self
            .displays
            .iter()
            .map(|display| display.id.clone())
            .collect();
        self.staged_modes.retain(|id, _| ids.contains(id));
        self.primary = self.primary.take().filter(|id| ids.contains(id));
        let selection_alive = self
            .selected
            .as_deref()
            .is_some_and(|id| ids.iter().any(|candidate| candidate == id));
        if !selection_alive {
            self.selected = self.displays.first().map(|display| display.id.clone());
        }
    }

    pub fn adopt_fresh(&mut self, fresh: &DisplayLayoutState) {
        self.bindings = fresh.bindings.clone();
        self.install_layout(fresh.displays.clone());
        self.modes = fresh.modes.clone();
        if fresh.error.is_none() {
            self.error = None;
        }
    }

    pub fn load_modes(&mut self, payload: &Value) {
        let rows = payload.as_array().cloned().unwrap_or_default();
        self.modes = rows.iter().filter_map(ModeOption::from_row).collect();
    }

    pub fn modes_for(&self, display_id: &str) -> Vec<ModeOption> {
        self.modes
            .iter()
            .filter(|option| option.display_id == display_id)
            .cloned()
            .collect()
    }

    pub fn modes_for_selected(&self) -> Vec<ModeOption> {
        match self.selected.as_deref() {
            Some(id) => self.modes_for(id),
            None => Vec::new(),
        }
    }

    pub fn modes_writable(&self) -> bool {
        self.modes.iter().any(|option| option.writable)
    }

    pub fn select(&mut self, id: &str) -> bool {
        if !self.displays.iter().any(|display| display.id == id) {
            return false;
        }
        self.selected = Some(id.to_string());
        true
    }

    pub fn cycle(&mut self, step: i32) -> bool {
        if self.displays.is_empty() {
            return false;
        }
        let index = self
            .selected
            .as_deref()
            .and_then(|id| self.displays.iter().position(|display| display.id == id))
            .unwrap_or(0);
        let count = self.displays.len() as i32;
        let next = (index as i32 + step).rem_euclid(count);
        self.selected = Some(self.displays[next as usize].id.clone());
        true
    }

    pub fn begin_drag(&mut self, id: &str) -> bool {
        let Some((display_id, x, y)) = self
            .displays
            .iter()
            .find(|display| display.id == id)
            .map(|display| (display.id.clone(), display.x, display.y))
        else {
            return false;
        };
        self.selected = Some(display_id.clone());
        self.edit = Some(StagedEdit {
            id: display_id,
            x,
            y,
            origin: (x, y),
            kind: EditKind::Drag,
            active: true,
            moved: false,
        });
        true
    }

    pub fn drag_delta(&mut self, dx: f32, dy: f32) -> bool {
        let Some(fit) = self.fit() else {
            return false;
        };
        let Some(edit) = self
            .edit
            .as_ref()
            .filter(|edit| edit.kind == EditKind::Drag)
        else {
            return false;
        };
        let id = edit.id.clone();
        let (width, height) = self.effective_size(&id);
        let candidate = Rect {
            x: edit.origin.0.saturating_add(fit.layout_delta_x(dx)),
            y: edit.origin.1.saturating_add(fit.layout_delta_y(dy)),
            width,
            height,
        };
        let others = self.other_rects(&id);
        let snapped = snap_rect(candidate, &others, snap_threshold(fit.scale));
        let Some(edit) = self.edit.as_mut() else {
            return false;
        };
        edit.x = snapped.x.clamp(SCREEN_MIN, SCREEN_MAX);
        edit.y = snapped.y.clamp(SCREEN_MIN, SCREEN_MAX);
        edit.moved = edit.moved || (edit.x, edit.y) != edit.origin;
        true
    }

    pub fn end_drag(&mut self) -> bool {
        let Some(edit) = self.edit.as_mut() else {
            return false;
        };
        if edit.kind != EditKind::Drag {
            return false;
        }
        if !edit.moved {
            self.edit = None;
            return false;
        }
        edit.active = false;
        true
    }

    pub fn nudge(&mut self, dx: i32, dy: i32) -> bool {
        let Some(display) = self.selected().cloned() else {
            return false;
        };
        let mut edit = match self.edit.take() {
            Some(edit) if edit.id == display.id && edit.kind == EditKind::Nudge => edit,
            _ => StagedEdit {
                id: display.id.clone(),
                x: display.x,
                y: display.y,
                origin: (display.x, display.y),
                kind: EditKind::Nudge,
                active: false,
                moved: false,
            },
        };
        edit.x = edit.x.saturating_add(dx).clamp(SCREEN_MIN, SCREEN_MAX);
        edit.y = edit.y.saturating_add(dy).clamp(SCREEN_MIN, SCREEN_MAX);
        edit.moved = edit.moved || (edit.x, edit.y) != edit.origin;
        self.edit = Some(edit);
        true
    }

    pub fn has_staged_edits(&self) -> bool {
        self.edit.as_ref().is_some_and(|edit| edit.moved)
            || !self.staged_modes.is_empty()
            || self.primary.is_some()
    }

    pub fn discard_staged(&mut self) -> bool {
        let staged = self.has_staged_edits();
        self.abort_commit();
        staged
    }

    pub fn arrange_intent(&self) -> Option<DisplayLayoutIntent> {
        let primary = self.primary_id()?.to_string();
        let placements: Vec<Placement> = self
            .displays
            .iter()
            .map(|display| {
                let (x, y) = self.effective_position(&display.id);
                Placement {
                    id: display.id.clone(),
                    x,
                    y,
                }
            })
            .collect();
        Some(DisplayLayoutIntent::Arrange {
            placements: normalize_placements(&placements),
            primary,
        })
    }

    pub fn set_primary(&mut self) -> bool {
        let Some(id) = self.selected_id().map(str::to_string) else {
            return false;
        };
        if self.primary_id() == Some(id.as_str()) {
            return false;
        }
        self.primary = Some(id);
        true
    }

    pub fn stage_mode(&mut self, option: &ModeOption) -> Option<DisplayLayoutIntent> {
        if !option.selectable || !self.modes_writable() {
            return None;
        }
        self.staged_modes.insert(
            option.display_id.clone(),
            StagedMode {
                display_id: option.display_id.clone(),
                token: option.token,
                width: option.width,
                height: option.height,
                refresh_hz: option.refresh_hz,
            },
        );
        self.resolve_staged_overlaps(&option.display_id);
        Some(DisplayLayoutIntent::SetMode {
            display_id: option.display_id.clone(),
            token: option.token,
            width: option.width,
            height: option.height,
            refresh_hz: option.refresh_hz,
        })
    }

    pub fn staged_mode_matches(&self, display_id: &str, token: u64) -> bool {
        self.staged_mode(display_id)
            .is_some_and(|staged| staged.token == token)
    }

    pub fn pending_mode_intents(&self) -> Vec<DisplayLayoutIntent> {
        self.staged_modes
            .values()
            .map(|staged| DisplayLayoutIntent::SetMode {
                display_id: staged.display_id.clone(),
                token: staged.token,
                width: staged.width,
                height: staged.height,
                refresh_hz: staged.refresh_hz,
            })
            .collect()
    }

    pub fn finish_commit(&mut self) {
        for display in &mut self.displays {
            if let Some(edit) = self.edit.as_ref().filter(|edit| edit.id == display.id) {
                display.x = edit.x;
                display.y = edit.y;
            }
            if let Some(staged) = self.staged_modes.get(&display.id) {
                display.width = i32::try_from(staged.width).unwrap_or(display.width);
                display.height = i32::try_from(staged.height).unwrap_or(display.height);
                display.refresh_hz = staged.refresh_hz;
            }
        }
        self.edit = None;
        self.staged_modes.clear();
        self.primary = None;
        self.editing = false;
    }

    pub fn abort_commit(&mut self) {
        self.edit = None;
        self.staged_modes.clear();
        self.primary = None;
        self.editing = false;
    }

    pub fn rect_of(&self, display: &Display) -> Rect {
        self.effective_rect(&display.id)
    }

    pub fn conflicts(&self, id: &str) -> bool {
        let rect = self.effective_rect(id);
        self.displays
            .iter()
            .any(|display| display.id != id && self.effective_rect(&display.id).overlaps(&rect))
    }

    pub fn issues(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        let mut seen = BTreeMap::new();
        for display in &self.displays {
            *seen.entry(display.id.clone()).or_insert(0usize) += 1;
        }
        for (id, count) in &seen {
            if *count > 1 {
                issues.push(Issue::DuplicateDisplay(id.clone()));
            }
        }
        let primaries = self
            .displays
            .iter()
            .filter(|display| self.is_primary(display))
            .count();
        if primaries != 1 {
            issues.push(Issue::PrimaryCount(primaries));
        }
        for display in &self.displays {
            let rect = self.effective_rect(&display.id);
            if !(SCREEN_MIN..=SCREEN_MAX).contains(&rect.x)
                || !(SCREEN_MIN..=SCREEN_MAX).contains(&rect.y)
            {
                issues.push(Issue::OutOfRange(display.id.clone()));
            }
        }
        for (index, display) in self.displays.iter().enumerate() {
            let rect = self.effective_rect(&display.id);
            for other in self.displays.iter().skip(index + 1) {
                if rect.overlaps(&self.effective_rect(&other.id)) {
                    issues.push(Issue::Overlap(display.id.clone(), other.id.clone()));
                }
            }
        }
        issues
    }

    pub fn is_committable(&self) -> bool {
        for issue in self.issues() {
            if matches!(
                issue,
                Issue::DuplicateDisplay(_) | Issue::PrimaryCount(_) | Issue::OutOfRange(_)
            ) {
                return false;
            }
        }
        true
    }

    fn effective_position(&self, id: &str) -> (i32, i32) {
        if let Some(edit) = self.edit.as_ref().filter(|edit| edit.id == id) {
            return (edit.x, edit.y);
        }
        match self.displays.iter().find(|display| display.id == id) {
            Some(display) => (display.x, display.y),
            None => (0, 0),
        }
    }

    fn effective_size(&self, id: &str) -> (i32, i32) {
        if let Some(staged) = self.staged_mode(id) {
            return (
                i32::try_from(staged.width).unwrap_or(0),
                i32::try_from(staged.height).unwrap_or(0),
            );
        }
        match self.displays.iter().find(|display| display.id == id) {
            Some(display) => (display.width, display.height),
            None => (0, 0),
        }
    }

    fn effective_rect(&self, id: &str) -> Rect {
        let (x, y) = self.effective_position(id);
        let (width, height) = self.effective_size(id);
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn other_rects(&self, id: &str) -> Vec<Rect> {
        self.displays
            .iter()
            .filter(|display| display.id != id)
            .map(|display| self.effective_rect(&display.id))
            .collect()
    }

    fn resolve_staged_overlaps(&mut self, id: &str) {
        let limit = self.displays.len().saturating_mul(2).max(1);
        for _ in 0..limit {
            if !self.settle_staged_overlap(id) {
                break;
            }
        }
    }

    fn settle_staged_overlap(&mut self, id: &str) -> bool {
        let moving = self.effective_rect(id);
        let Some((other_id, other)) = self
            .displays
            .iter()
            .filter(|display| display.id != id)
            .map(|display| (display.id.clone(), self.effective_rect(&display.id)))
            .find(|(_, other)| moving.overlaps(other))
        else {
            return false;
        };
        let Some((dx, dy)) = overlap_shift(moving, other) else {
            return false;
        };
        let moving_target = (moving.x.saturating_add(dx), moving.y.saturating_add(dy));
        if in_screen_range(moving_target.0) && in_screen_range(moving_target.1) {
            self.shift_staged(id, dx, dy);
            return true;
        }
        let other_target = (other.x.saturating_sub(dx), other.y.saturating_sub(dy));
        if in_screen_range(other_target.0) && in_screen_range(other_target.1) {
            self.shift_staged(&other_id, -dx, -dy);
            return true;
        }
        false
    }

    fn shift_staged(&mut self, id: &str, dx: i32, dy: i32) {
        if let Some(edit) = self.edit.as_mut().filter(|edit| edit.id == id) {
            let x = edit.x.saturating_add(dx).clamp(SCREEN_MIN, SCREEN_MAX);
            let y = edit.y.saturating_add(dy).clamp(SCREEN_MIN, SCREEN_MAX);
            edit.x = x;
            edit.y = y;
            edit.moved = edit.moved || (x, y) != edit.origin;
            return;
        }
        let Some((origin_x, origin_y)) = self
            .displays
            .iter()
            .find(|display| display.id == id)
            .map(|display| (display.x, display.y))
        else {
            return;
        };
        let x = origin_x.saturating_add(dx).clamp(SCREEN_MIN, SCREEN_MAX);
        let y = origin_y.saturating_add(dy).clamp(SCREEN_MIN, SCREEN_MAX);
        self.edit = Some(StagedEdit {
            id: id.to_string(),
            x,
            y,
            origin: (origin_x, origin_y),
            kind: EditKind::Nudge,
            active: false,
            moved: (x, y) != (origin_x, origin_y),
        });
    }
}

fn in_screen_range(value: i32) -> bool {
    (SCREEN_MIN..=SCREEN_MAX).contains(&value)
}

fn overlap_shift(moving: Rect, other: Rect) -> Option<(i32, i32)> {
    let overlap_x = moving
        .right()
        .min(other.right())
        .saturating_sub(moving.x.max(other.x));
    let overlap_y = moving
        .bottom()
        .min(other.bottom())
        .saturating_sub(moving.y.max(other.y));
    if overlap_x <= 0 || overlap_y <= 0 {
        return None;
    }
    if overlap_x <= overlap_y {
        let left = moving.right().saturating_sub(other.x);
        let right = other.right().saturating_sub(moving.x);
        if left <= right {
            Some((-left, 0))
        } else {
            Some((right, 0))
        }
    } else {
        let up = moving.bottom().saturating_sub(other.y);
        let down = other.bottom().saturating_sub(moving.y);
        if up <= down {
            Some((0, -up))
        } else {
            Some((0, down))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(id: &str, x: i32, y: i32, width: i32, height: i32, primary: bool) -> Display {
        Display {
            id: id.to_string(),
            connector: format!("card0-{id}"),
            x,
            y,
            width,
            height,
            refresh_hz: 60,
            primary,
        }
    }

    fn bindings() -> DisplayLayoutBindings {
        DisplayLayoutBindings::new(
            "layout",
            Some("modes".to_string()),
            "arrange",
            Some("set_mode".to_string()),
        )
    }

    fn state() -> DisplayLayoutState {
        DisplayLayoutState::new(bindings())
    }

    fn two_displays() -> DisplayLayoutState {
        let mut layout = state();
        layout.set_viewport(720.0, 368.0, 16.0);
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 3840,
                "height": 2160,
                "refresh_hz": 60,
                "primary": true,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 3840,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "refresh_hz": 60,
                "primary": false,
            }
        ]));
        layout
    }

    #[derive(serde::Deserialize)]
    struct DaemonArrangePlacement {
        id: String,
        x: i32,
        y: i32,
    }

    #[derive(serde::Deserialize)]
    struct DaemonArrangeInput {
        placements: Vec<DaemonArrangePlacement>,
        #[serde(default)]
        primary: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct DaemonSetModeInput {
        id: String,
        #[serde(default)]
        token: Option<u64>,
        width: u32,
        height: u32,
        #[serde(default)]
        refresh: Option<u32>,
    }

    fn parse_arrange_input(input: &Value) -> Result<DaemonArrangeInput, String> {
        let parsed: DaemonArrangeInput = serde_json::from_value(input.clone())
            .map_err(|error| format!("arrange placements are invalid: {error}"))?;
        if parsed.placements.is_empty() {
            return Err("arrange input requires at least one placement".to_string());
        }
        Ok(parsed)
    }

    fn parse_set_mode_input(input: &Value) -> Result<DaemonSetModeInput, String> {
        let parsed: DaemonSetModeInput = serde_json::from_value(input.clone())
            .map_err(|error| format!("set_mode input is invalid: {error}"))?;
        if parsed.width == 0 || parsed.height == 0 {
            return Err("set_mode input requires a positive width".to_string());
        }
        if parsed.refresh == Some(0) {
            return Err("set_mode refresh must be a positive integer".to_string());
        }
        Ok(parsed)
    }

    fn positions(placements: &[DaemonArrangePlacement]) -> Vec<(String, i32, i32)> {
        placements
            .iter()
            .map(|placement| (placement.id.clone(), placement.x, placement.y))
            .collect()
    }

    fn staged_positions(layout: &DisplayLayoutState) -> Vec<(String, i32, i32)> {
        layout
            .displays()
            .iter()
            .map(|display| {
                let (x, y) = layout.effective_position(&display.id);
                (display.id.clone(), x, y)
            })
            .collect()
    }

    fn pairwise_offsets(placements: &[(String, i32, i32)]) -> Vec<(String, String, i32, i32)> {
        let mut offsets = Vec::new();
        for (index, first) in placements.iter().enumerate() {
            for second in placements.iter().skip(index + 1) {
                offsets.push((
                    first.0.clone(),
                    second.0.clone(),
                    second.1 - first.1,
                    second.2 - first.2,
                ));
            }
        }
        offsets
    }

    #[test]
    fn fit_returns_none_for_an_empty_layout() {
        assert!(fit(&[], 400.0, 200.0, 16.0).is_none());
    }

    #[test]
    fn fit_keeps_negative_origins_ordered_and_padded() {
        let displays = vec![
            display("left", -1920, 0, 1920, 1080, false),
            display("right", 0, 0, 1920, 1080, true),
        ];
        let fit = fit(&displays, 400.0, 200.0, 16.0).expect("fit");
        assert_eq!(fit.min_x, -1920);
        assert_eq!(fit.min_y, 0);
        let left = fit.to_client_x(-1920);
        let right = fit.to_client_x(0);
        assert!((left - 16.0).abs() <= 1.0, "left {left}");
        assert!(right > left);
        assert!(right <= 400.0 - 16.0 + 1.0, "right {right}");
        assert!(fit.to_client_y(0) >= 16.0 - 1.0);
        assert!(fit.to_client_x(0) + fit.to_client_width(1920) <= 400.0 - 16.0 + 1.0);
    }

    #[test]
    fn fit_shrinks_a_portrait_beside_a_landscape_with_one_factor() {
        let displays = vec![
            display("portrait", 0, 0, 1080, 1920, true),
            display("landscape", 1080, 0, 1920, 1080, false),
        ];
        let fit = fit(&displays, 640.0, 360.0, 16.0).expect("fit");
        let portrait_width = fit.to_client_width(1080);
        let portrait_height = fit.to_client_height(1920);
        let landscape_width = fit.to_client_width(1920);
        let landscape_height = fit.to_client_height(1080);
        assert!((portrait_width - 1080.0 * fit.scale).abs() <= 1.0);
        assert!((portrait_height - 1920.0 * fit.scale).abs() <= 1.0);
        assert!((landscape_width - 1920.0 * fit.scale).abs() <= 1.0);
        assert!((landscape_height - 1080.0 * fit.scale).abs() <= 1.0);
        assert!(fit.to_client_width(3000) <= 640.0 - 32.0 + 1.0);
        assert!(fit.to_client_height(1920) <= 360.0 - 32.0 + 1.0);
    }

    #[test]
    fn fit_never_grows_a_small_layout_past_one_to_one() {
        let displays = vec![display("tiny", 0, 0, 640, 480, true)];
        let fit = fit(&displays, 800.0, 600.0, 16.0).expect("fit");
        assert_eq!(fit.scale, 1.0);
        assert_eq!(fit.to_client_width(640), 640.0);
        assert_eq!(fit.to_client_height(480), 480.0);
    }

    #[test]
    fn delta_conversion_round_trips_within_a_pixel() {
        let layout = two_displays();
        let fit = layout.fit().expect("fit");
        assert!(fit.scale < 1.0);
        for client in [1.0_f32, 3.0, 7.0, 40.0, 133.0, 300.0] {
            let screen = fit.layout_delta_x(client);
            let back = fit.to_client_delta_x(screen as f32);
            assert!(
                (back - client).abs() <= 1.0,
                "client {client} screen {screen} back {back}"
            );
        }
        for screen in [1_i32, 2, 5, 9, 33] {
            let drawn = fit.to_client_delta_x(screen as f32);
            let exact = screen as f32 * fit.scale;
            assert!(
                (drawn - exact).abs() <= 0.5,
                "screen {screen} drawn {drawn} exact {exact}"
            );
        }
    }

    #[test]
    fn drag_delta_converts_client_pixels_through_the_inverse_scale() {
        let mut layout = two_displays();
        let fit = layout.fit().expect("fit");
        assert!(layout.begin_drag("beta"));
        assert!(layout.drag_delta(1000.0, 0.0));
        assert_eq!(
            layout.effective_position("beta"),
            (3840 + fit.layout_delta_x(1000.0), 0)
        );
        assert!(layout.end_drag());
    }

    #[test]
    fn a_drag_within_ten_screen_pixels_snaps_to_exact_contact_at_a_small_scale() {
        let mut layout = two_displays();
        let fit = layout.fit().expect("fit");
        assert!(fit.scale < 0.5);
        let drag = fit.layout_delta_x(6.0);
        assert!(drag > 0);
        assert!(drag < snap_threshold(fit.scale));
        assert!(layout.begin_drag("beta"));
        assert!(layout.drag_delta(6.0, 0.0));
        let edit = layout.edit().expect("staged drag");
        assert_eq!(edit.x, layout.effective_rect("alpha").right());
        assert!(!edit.moved);
        assert!(!layout.end_drag());
    }

    #[test]
    fn drag_without_movement_selects_and_does_not_commit() {
        let mut layout = two_displays();
        assert_eq!(layout.selected_id(), Some("alpha"));
        assert!(layout.begin_drag("beta"));
        assert_eq!(layout.selected_id(), Some("beta"));
        assert!(!layout.end_drag());
        assert!(layout.edit().is_none());
        assert_eq!(layout.effective_position("beta"), (3840, 0));
    }

    #[test]
    fn snap_pulls_edges_to_exact_contact_within_the_threshold() {
        let moving = Rect {
            x: 100,
            y: 0,
            width: 100,
            height: 100,
        };
        let other = Rect {
            x: 203,
            y: 0,
            width: 100,
            height: 100,
        };
        let snapped = snap_rect(moving, &[other], snap_threshold(1.0));
        assert_eq!(snapped.x, 103);
        assert_eq!(snapped.right(), other.x);
        assert!(!snapped.overlaps(&other));

        let overlapping = Rect {
            x: 104,
            y: 0,
            width: 100,
            height: 100,
        };
        let snapped_overlap = snap_rect(overlapping, &[other], snap_threshold(1.0));
        assert_eq!(snapped_overlap.right(), other.x);
        assert!(!snapped_overlap.overlaps(&other));
    }

    #[test]
    fn snap_leaves_a_layout_alone_beyond_the_threshold() {
        let moving = Rect {
            x: 100,
            y: 100,
            width: 100,
            height: 100,
        };
        let other = Rect {
            x: 211,
            y: 100,
            width: 100,
            height: 100,
        };
        assert_eq!(snap_rect(moving, &[other], snap_threshold(1.0)), moving);
    }

    #[test]
    fn snap_matches_centers_within_the_threshold() {
        let other = Rect {
            x: 60,
            y: 300,
            width: 180,
            height: 100,
        };
        let aligned = Rect {
            x: 100,
            y: 0,
            width: 100,
            height: 100,
        };
        assert_eq!(snap_rect(aligned, &[other], snap_threshold(1.0)), aligned);
        let shifted = Rect {
            x: 103,
            y: 0,
            width: 100,
            height: 100,
        };
        let snapped = snap_rect(shifted, &[other], snap_threshold(1.0));
        assert_eq!(
            snapped.x + snapped.width / 2,
            other.x + other.width / 2,
            "centers must share a coordinate"
        );
    }

    #[test]
    fn snap_threshold_tracks_screen_pixels_across_scales() {
        for scale in [1.0_f32, 0.5, 0.25, 0.125] {
            let threshold = snap_threshold(scale);
            let covered = threshold as f32 * scale;
            assert!(
                (covered - SNAP_THRESHOLD_PX).abs() <= scale,
                "scale {scale} threshold {threshold} covers {covered} screen pixels"
            );
            assert!(threshold >= 1);
        }
    }

    #[test]
    fn touching_edges_do_not_overlap() {
        let left = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let right = Rect {
            x: 100,
            y: 0,
            width: 100,
            height: 100,
        };
        let below = Rect {
            x: 0,
            y: 100,
            width: 100,
            height: 100,
        };
        assert!(!left.overlaps(&right));
        assert!(!right.overlaps(&left));
        assert!(!left.overlaps(&below));
    }

    #[test]
    fn one_pixel_of_overlap_is_detected() {
        let left = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let overlapping = Rect {
            x: 99,
            y: 0,
            width: 100,
            height: 100,
        };
        assert!(left.overlaps(&overlapping));
        assert!(overlapping.overlaps(&left));
    }

    fn growth_layout(
        alpha: (i32, i32, i32, i32),
        beta: (i32, i32, i32, i32),
    ) -> DisplayLayoutState {
        let mut layout = state();
        layout.set_viewport(720.0, 368.0, 16.0);
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": alpha.0,
                "y": alpha.1,
                "width": alpha.2,
                "height": alpha.3,
                "refresh_hz": 60,
                "primary": true,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": beta.0,
                "y": beta.1,
                "width": beta.2,
                "height": beta.3,
                "refresh_hz": 60,
                "primary": false,
            }
        ]));
        layout
    }

    fn stage_growth(
        layout: &mut DisplayLayoutState,
        token: u64,
        width: u32,
        height: u32,
    ) -> DisplayLayoutIntent {
        layout.load_modes(&json!([{
            "id": format!("alpha#{token}"),
            "display_id": "alpha",
            "connector": "card0-DP-1",
            "token": token,
            "width": width,
            "height": height,
            "refresh_hz": 60,
            "label": format!("{width}x{height}@60"),
            "detail": "available mode",
            "current": false,
            "writable": true,
            "selectable": true,
        }]));
        let option = layout.modes_for("alpha").remove(0);
        layout.stage_mode(&option).expect("staged mode")
    }

    #[test]
    fn growth_into_a_right_neighbour_shifts_the_changed_display_left_to_exact_contact() {
        let mut layout = growth_layout((0, 0, 1920, 1080), (1920, 0, 1280, 720));
        assert!(layout.issues().is_empty());
        let intent = stage_growth(&mut layout, 7, 2560, 1440);
        assert_eq!(intent.action(), "set_mode");
        assert_eq!(layout.effective_position("alpha"), (-640, 0));
        assert_eq!(
            layout.effective_rect("alpha").right(),
            layout.effective_rect("beta").x
        );
        assert!(layout.issues().is_empty());
        assert!(!layout.conflicts("alpha"));
        assert!(!layout.conflicts("beta"));
        assert!(layout.is_committable());
    }

    #[test]
    fn growth_into_a_left_neighbour_shifts_the_changed_display_right_to_exact_contact() {
        let mut layout = growth_layout((300, 0, 600, 600), (0, 800, 800, 500));
        let intent = stage_growth(&mut layout, 11, 600, 1400);
        assert_eq!(intent.action(), "set_mode");
        assert_eq!(layout.effective_position("alpha"), (800, 0));
        assert_eq!(
            layout.effective_rect("alpha").x,
            layout.effective_rect("beta").right()
        );
        assert!(layout.issues().is_empty());
        assert!(layout.is_committable());
    }

    #[test]
    fn growth_with_free_space_leaves_every_display_in_place() {
        let mut layout = growth_layout((0, 0, 1920, 1080), (3000, 0, 1280, 720));
        let intent = stage_growth(&mut layout, 13, 2560, 1440);
        assert_eq!(intent.action(), "set_mode");
        assert_eq!(layout.effective_position("alpha"), (0, 0));
        assert_eq!(layout.effective_position("beta"), (3000, 0));
        assert!(layout.edit().is_none());
        assert!(layout.issues().is_empty());
    }

    #[test]
    fn a_shifted_mode_change_produces_a_valid_arrange_payload() {
        let mut layout = growth_layout((0, 0, 1920, 1080), (1920, 0, 1280, 720));
        stage_growth(&mut layout, 7, 2560, 1440);
        let intent = layout.arrange_intent().expect("arrange intent");
        match parse_arrange_input(&intent.input()) {
            Ok(parsed) => {
                assert_eq!(parsed.primary.as_deref(), Some("alpha"));
                assert_eq!(parsed.placements.len(), 2);
                assert_eq!(
                    (
                        parsed.placements[0].id.as_str(),
                        parsed.placements[0].x,
                        parsed.placements[0].y
                    ),
                    ("alpha", 0, 0)
                );
                assert_eq!(
                    (
                        parsed.placements[1].id.as_str(),
                        parsed.placements[1].x,
                        parsed.placements[1].y
                    ),
                    ("beta", 2560, 0)
                );
            }
            Err(error) => panic!("the arrange payload must parse: {error}"),
        }
    }

    #[test]
    fn arrange_payload_round_trips_through_the_daemon_parser() {
        let mut layout = two_displays();
        assert!(layout.select("beta"));
        assert!(layout.nudge(-120, 0));
        let intent = layout.arrange_intent().expect("arrange intent");
        assert_eq!(intent.action(), "arrange");
        match parse_arrange_input(&intent.input()) {
            Ok(parsed) => {
                assert_eq!(parsed.primary.as_deref(), Some("alpha"));
                assert_eq!(parsed.placements.len(), 2);
                assert_eq!(
                    (
                        parsed.placements[0].id.as_str(),
                        parsed.placements[0].x,
                        parsed.placements[0].y
                    ),
                    ("alpha", 0, 0)
                );
                assert_eq!(
                    (
                        parsed.placements[1].id.as_str(),
                        parsed.placements[1].x,
                        parsed.placements[1].y
                    ),
                    ("beta", 3720, 0)
                );
            }
            Err(error) => panic!("the arrange payload must parse: {error}"),
        }
    }

    #[test]
    fn arrange_payload_carries_every_display_and_one_primary() {
        let layout = two_displays();
        let intent = layout.arrange_intent().expect("arrange intent");
        match parse_arrange_input(&intent.input()) {
            Ok(parsed) => {
                let ids: Vec<&str> = parsed
                    .placements
                    .iter()
                    .map(|row| row.id.as_str())
                    .collect();
                assert_eq!(ids, vec!["alpha", "beta"]);
                assert_eq!(parsed.primary.as_deref(), Some("alpha"));
            }
            Err(error) => panic!("the arrange payload must parse: {error}"),
        }
    }

    #[test]
    fn arrange_payload_compacts_a_negative_origin_and_keeps_pairwise_deltas() {
        let mut layout = state();
        layout.set_viewport(720.0, 368.0, 16.0);
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": -1920,
                "y": -1080,
                "width": 1920,
                "height": 1080,
                "refresh_hz": 60,
                "primary": true,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 0,
                "y": 0,
                "width": 1280,
                "height": 720,
                "refresh_hz": 60,
                "primary": false,
            },
            {
                "id": "gamma",
                "connector": "card0-DP-3",
                "x": -4480,
                "y": 720,
                "width": 2560,
                "height": 1440,
                "refresh_hz": 60,
                "primary": false,
            }
        ]));
        let staged = staged_positions(&layout);
        let intent = layout.arrange_intent().expect("arrange intent");
        let parsed = match parse_arrange_input(&intent.input()) {
            Ok(parsed) => parsed,
            Err(error) => panic!("the arrange payload must parse: {error}"),
        };
        assert_eq!(parsed.placements.iter().map(|row| row.x).min(), Some(0));
        assert_eq!(parsed.placements.iter().map(|row| row.y).min(), Some(0));
        let normalized = positions(&parsed.placements);
        assert_eq!(pairwise_offsets(&normalized), pairwise_offsets(&staged));
        assert_eq!(
            normalized,
            vec![
                ("alpha".to_string(), 2560, 0),
                ("beta".to_string(), 4480, 1080),
                ("gamma".to_string(), 0, 1800),
            ]
        );
    }

    #[test]
    fn arrange_payload_leaves_a_non_negative_origin_unchanged() {
        let mut layout = state();
        layout.set_viewport(720.0, 368.0, 16.0);
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 320,
                "y": 160,
                "width": 1920,
                "height": 1080,
                "refresh_hz": 60,
                "primary": true,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 2240,
                "y": 160,
                "width": 1280,
                "height": 720,
                "refresh_hz": 60,
                "primary": false,
            }
        ]));
        let staged = staged_positions(&layout);
        let intent = layout.arrange_intent().expect("arrange intent");
        let parsed = match parse_arrange_input(&intent.input()) {
            Ok(parsed) => parsed,
            Err(error) => panic!("the arrange payload must parse: {error}"),
        };
        assert_eq!(positions(&parsed.placements), staged);
    }

    #[test]
    fn arrange_payload_keeps_the_primary_across_normalization() {
        let mut layout = state();
        layout.set_viewport(720.0, 368.0, 16.0);
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": -2560,
                "y": -1440,
                "width": 1920,
                "height": 1080,
                "refresh_hz": 60,
                "primary": false,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": -1280,
                "y": 0,
                "width": 1280,
                "height": 720,
                "refresh_hz": 60,
                "primary": true,
            }
        ]));
        let intent = layout.arrange_intent().expect("arrange intent");
        let parsed = match parse_arrange_input(&intent.input()) {
            Ok(parsed) => parsed,
            Err(error) => panic!("the arrange payload must parse: {error}"),
        };
        assert_eq!(parsed.primary.as_deref(), Some("beta"));
        assert_eq!(
            positions(&parsed.placements),
            vec![
                ("alpha".to_string(), 0, 0),
                ("beta".to_string(), 1280, 1440)
            ]
        );
    }

    #[test]
    fn arrange_payload_normalizes_a_lone_negative_display_to_the_origin() {
        let mut layout = state();
        layout.set_viewport(720.0, 368.0, 16.0);
        layout.load_layout(&json!([{
            "id": "solo",
            "connector": "card0-DP-1",
            "x": -1280,
            "y": -720,
            "width": 1280,
            "height": 720,
            "refresh_hz": 60,
            "primary": true,
        }]));
        let intent = layout.arrange_intent().expect("arrange intent");
        let parsed = match parse_arrange_input(&intent.input()) {
            Ok(parsed) => parsed,
            Err(error) => panic!("the arrange payload must parse: {error}"),
        };
        assert_eq!(parsed.primary.as_deref(), Some("solo"));
        assert_eq!(
            positions(&parsed.placements),
            vec![("solo".to_string(), 0, 0)]
        );
    }

    #[test]
    fn primary_intent_keeps_the_placements_and_switches_the_primary() {
        let mut layout = two_displays();
        assert!(layout.select("beta"));
        assert!(layout.set_primary());
        let intent = layout.arrange_intent().expect("arrange intent");
        match parse_arrange_input(&intent.input()) {
            Ok(parsed) => {
                assert_eq!(parsed.primary.as_deref(), Some("beta"));
                assert_eq!(
                    (
                        parsed.placements[0].x,
                        parsed.placements[0].y,
                        parsed.placements[1].x,
                        parsed.placements[1].y
                    ),
                    (0, 0, 3840, 0)
                );
            }
            Err(error) => panic!("a primary change must commit as arrange: {error}"),
        }
    }

    #[test]
    fn set_mode_payload_round_trips_through_the_daemon_parser() {
        let mut layout = two_displays();
        layout.load_modes(&json!([{
            "id": "alpha#4",
            "display_id": "alpha",
            "connector": "card0-DP-1",
            "token": 4,
            "width": 1280,
            "height": 720,
            "refresh_hz": 75,
            "label": "1280x720@75",
            "detail": "available mode",
            "current": false,
            "writable": true,
            "selectable": true,
        }]));
        let option = layout.modes_for("alpha").remove(0);
        let intent = layout.stage_mode(&option).expect("set mode intent");
        assert_eq!(intent.action(), "set_mode");
        assert!(layout.staged_mode_matches("alpha", 4));
        match parse_set_mode_input(&intent.input()) {
            Ok(parsed) => {
                assert_eq!(parsed.id, "alpha");
                assert_eq!(parsed.token, Some(4));
                assert_eq!(
                    (parsed.width, parsed.height, parsed.refresh),
                    (1280, 720, Some(75))
                );
            }
            Err(error) => panic!("the set_mode payload must parse: {error}"),
        }
    }

    #[test]
    fn set_mode_payload_serializes_an_unreadable_refresh_as_null() {
        let mut layout = two_displays();
        layout.load_modes(&json!([{
            "id": "alpha#9",
            "display_id": "alpha",
            "connector": "card0-DP-1",
            "token": 9,
            "width": 1280,
            "height": 720,
            "refresh_hz": 0,
            "label": "1280x720",
            "detail": "available mode",
            "current": false,
            "writable": true,
            "selectable": true,
        }]));
        let option = layout.modes_for("alpha").remove(0);
        let intent = layout.stage_mode(&option).expect("set mode intent");
        let input = intent.input();
        assert_eq!(input.get("refresh"), Some(&Value::Null));
        match parse_set_mode_input(&input) {
            Ok(parsed) => assert_eq!(parsed.refresh, None),
            Err(error) => panic!("a null refresh must parse: {error}"),
        }
    }

    #[test]
    fn nudge_steps_are_ten_pixels_and_one_with_shift() {
        assert_eq!(nudge_step(false), NUDGE_STEP);
        assert_eq!(nudge_step(true), NUDGE_STEP_FINE);
        let mut layout = two_displays();
        assert!(layout.select("beta"));
        assert!(layout.nudge(0, -nudge_step(true)));
        assert_eq!(layout.effective_position("beta"), (3840, -1));
        assert!(layout.nudge(nudge_step(false), 0));
        assert_eq!(layout.effective_position("beta"), (3850, -1));
    }

    #[test]
    fn enter_commits_a_pending_nudge_and_escape_reverts_it() {
        let mut layout = two_displays();
        assert!(layout.select("beta"));
        assert!(layout.nudge(-1, 0));
        let intent = layout.arrange_intent().expect("commit intent");
        match intent {
            DisplayLayoutIntent::Arrange { placements, .. } => {
                assert_eq!(placements[1].x, 3839)
            }
            DisplayLayoutIntent::SetMode { .. } => panic!("a nudge must commit as arrange"),
        }

        let mut reverted = two_displays();
        assert!(reverted.select("beta"));
        assert!(reverted.nudge(-1, 0));
        assert!(reverted.discard_staged());
        assert_eq!(reverted.effective_position("beta"), (3840, 0));
        assert!(!reverted.discard_staged());
    }

    #[test]
    fn nudges_stay_inside_the_x11_position_range() {
        let mut layout = two_displays();
        assert!(layout.select("beta"));
        assert!(layout.nudge(SCREEN_MIN - layout.effective_position("beta").0 - 5, 0));
        assert_eq!(layout.effective_position("beta").0, SCREEN_MIN);
        assert!(layout.nudge(SCREEN_MAX + 5 - SCREEN_MIN, 0));
        assert_eq!(layout.effective_position("beta").0, SCREEN_MAX);
    }

    #[test]
    fn mode_options_keep_colliding_labels_distinct() {
        let mut layout = state();
        layout.load_modes(&json!([
            {
                "id": "alpha#10",
                "display_id": "alpha",
                "connector": "card0-DP-1",
                "token": 10,
                "width": 1920,
                "height": 1080,
                "refresh_hz": 60,
                "label": "1920x1080@60",
                "detail": "current mode",
                "current": true,
                "writable": true,
                "selectable": false,
            },
            {
                "id": "alpha#11",
                "display_id": "alpha",
                "connector": "card0-DP-1",
                "token": 11,
                "width": 1920,
                "height": 1080,
                "refresh_hz": 60,
                "label": "1920x1080@60",
                "detail": "available mode",
                "current": false,
                "writable": true,
                "selectable": true,
            },
            {
                "id": "beta#12",
                "display_id": "beta",
                "connector": "card0-DP-2",
                "token": 12,
                "width": 1280,
                "height": 720,
                "refresh_hz": 60,
                "label": "1280x720@60",
                "detail": "available mode",
                "current": false,
                "writable": false,
                "selectable": false,
            }
        ]));
        let options = layout.modes_for("alpha");
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].label, options[1].label);
        assert_eq!(options[0].display_id, options[1].display_id);
        assert_ne!(options[0].token, options[1].token);
        assert_eq!(options.iter().filter(|option| option.selectable).count(), 1);
        assert_eq!(options.iter().filter(|option| option.current).count(), 1);
        assert!(layout.modes_for("gamma").is_empty());
    }

    #[test]
    fn an_unwritable_or_unselectable_mode_control_never_stages() {
        let mut layout = two_displays();
        layout.load_modes(&json!([{
            "id": "alpha#10",
            "display_id": "alpha",
            "connector": "card0-DP-1",
            "token": 10,
            "width": 1920,
            "height": 1080,
            "refresh_hz": 60,
            "label": "1920x1080@60",
            "detail": "current mode",
            "current": true,
            "writable": false,
            "selectable": false,
        }]));
        assert!(!layout.modes_writable());
        let option = layout.modes_for("alpha").remove(0);
        assert!(layout.stage_mode(&option).is_none());
        assert!(layout.staged_mode("alpha").is_none());
    }

    #[test]
    fn layout_issues_flag_duplicates_and_primary_count() {
        let mut layout = state();
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "primary": true,
            },
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "primary": false,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 1920,
                "y": 0,
                "width": 1280,
                "height": 720,
                "primary": true,
            }
        ]));
        let issues = layout.issues();
        assert!(issues
            .iter()
            .any(|issue| matches!(issue, Issue::DuplicateDisplay(id) if id == "alpha")));
        assert!(issues
            .iter()
            .any(|issue| matches!(issue, Issue::PrimaryCount(2))));
        assert!(!layout.is_committable());
        assert!(two_displays().is_committable());
    }

    #[test]
    fn layout_polls_revert_drag_edits_and_keep_pending_nudges() {
        let mut layout = two_displays();
        assert!(layout.begin_drag("beta"));
        assert!(layout.drag_delta(120.0, 0.0));
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 3840,
                "height": 2160,
                "primary": true,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 3840,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "primary": false,
            }
        ]));
        assert!(layout.edit().is_none());
        assert_eq!(layout.effective_position("beta"), (3840, 0));

        assert!(layout.select("beta"));
        assert!(layout.nudge(0, 12));
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 3840,
                "height": 2160,
                "primary": true,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 3840,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "primary": false,
            }
        ]));
        assert_eq!(layout.effective_position("beta"), (3840, 12));
    }

    #[test]
    fn a_released_drag_stays_staged_across_a_layout_poll() {
        let mut layout = two_displays();
        assert!(layout.begin_drag("beta"));
        assert!(layout.drag_delta(120.0, 0.0));
        let staged_x = layout.effective_position("beta").0;
        assert!(layout.end_drag());
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 3840,
                "height": 2160,
                "primary": true,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 3840,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "primary": false,
            }
        ]));
        assert!(layout.has_staged_edits());
        assert_eq!(layout.effective_position("beta").0, staged_x);
    }

    #[test]
    fn a_fresh_layout_keeps_staged_edits_for_displays_that_remain() {
        let mut layout = two_displays();
        assert!(layout.select("beta"));
        assert!(layout.nudge(0, 25));
        let mut fresh = state();
        fresh.set_viewport(720.0, 368.0, 16.0);
        fresh.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 3840,
                "height": 2160,
                "primary": true,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 3840,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "primary": false,
            }
        ]));
        layout.adopt_fresh(&fresh);
        assert_eq!(layout.effective_position("beta"), (3840, 25));
    }

    #[test]
    fn a_fresh_layout_drops_staged_edits_for_displays_that_left() {
        let mut layout = two_displays();
        assert!(layout.select("beta"));
        assert!(layout.nudge(0, 25));
        let mut fresh = state();
        fresh.load_layout(&json!([{
            "id": "alpha",
            "connector": "card0-DP-1",
            "x": 0,
            "y": 0,
            "width": 3840,
            "height": 2160,
            "primary": true,
        }]));
        layout.adopt_fresh(&fresh);
        assert!(layout.edit().is_none());
        assert_eq!(layout.displays().len(), 1);
    }

    #[test]
    fn load_layout_selects_the_first_display_and_keeps_a_live_selection() {
        let mut layout = state();
        layout.load_layout(&json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "primary": false,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 1920,
                "y": 0,
                "width": 1280,
                "height": 720,
                "primary": true,
            }
        ]));
        assert_eq!(layout.selected_id(), Some("alpha"));
        assert_eq!(layout.primary_id(), Some("beta"));
        assert!(layout.cycle(1));
        assert_eq!(layout.selected_id(), Some("beta"));
        assert!(layout.cycle(1));
        assert_eq!(layout.selected_id(), Some("alpha"));
        assert!(layout.cycle(-1));
        assert_eq!(layout.selected_id(), Some("beta"));
    }

    #[test]
    fn disconnect_clears_edits_and_selection() {
        let mut layout = two_displays();
        assert!(layout.select("beta"));
        assert!(layout.nudge(0, 4));
        layout.load_layout(&json!([]));
        assert!(layout.displays().is_empty());
        assert!(layout.selected_id().is_none());
        assert!(layout.edit().is_none());
        assert!(layout.arrange_intent().is_none());
        assert!(layout.fit().is_none());
    }

    #[test]
    fn abort_commit_reverts_every_local_edit() {
        let mut layout = two_displays();
        layout.load_modes(&json!([{
            "id": "beta#5",
            "display_id": "beta",
            "connector": "card0-DP-2",
            "token": 5,
            "width": 1280,
            "height": 720,
            "refresh_hz": 60,
            "label": "1280x720@60",
            "detail": "available mode",
            "current": false,
            "writable": true,
            "selectable": true,
        }]));
        assert!(layout.select("beta"));
        assert!(layout.nudge(-30, 0));
        let option = layout.modes_for("beta").remove(0);
        assert!(layout.stage_mode(&option).is_some());
        assert!(layout.set_primary());
        layout.abort_commit();
        assert!(layout.edit().is_none());
        assert!(layout.staged_mode("beta").is_none());
        assert_eq!(layout.primary_id(), Some("alpha"));
        assert_eq!(layout.effective_position("beta"), (3840, 0));
    }
}
