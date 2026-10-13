use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use qol_platform::native::com::{Apartment, ComApartment};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationSelectionItemPattern,
    TreeScope_Descendants, UIA_ControlTypePropertyId, UIA_SelectionItemPatternId,
    UIA_TabItemControlTypeId,
};

const MARK_TIMEOUT: Duration = Duration::from_millis(1500);
const MARK_POLL: Duration = Duration::from_millis(50);

struct TabStrip {
    automation: IUIAutomation,
    window: IUIAutomationElement,
}

impl TabStrip {
    fn open(window: u32) -> Option<TabStrip> {
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()?;
        let handle = HWND(window as usize as *mut c_void);
        let window = unsafe { automation.ElementFromHandle(handle) }.ok()?;
        Some(TabStrip { automation, window })
    }

    fn tabs(&self) -> Vec<(IUIAutomationElement, String)> {
        let kind = int_variant(UIA_TabItemControlTypeId.0);
        let found = unsafe {
            self.automation
                .CreatePropertyCondition(UIA_ControlTypePropertyId, &kind)
                .and_then(|condition| self.window.FindAll(TreeScope_Descendants, &condition))
        };
        let Ok(found) = found else {
            return Vec::new();
        };
        let count = unsafe { found.Length() }.unwrap_or(0);
        (0..count)
            .filter_map(|index| {
                let tab = unsafe { found.GetElement(index) }.ok()?;
                let name = unsafe { tab.CurrentName() }
                    .map(|name| name.to_string())
                    .unwrap_or_default();
                Some((tab, name))
            })
            .collect()
    }

    fn selected(&self) -> Option<IUIAutomationElement> {
        self.tabs().into_iter().map(|(tab, _)| tab).find(|tab| {
            selection(tab).is_some_and(|pattern| {
                unsafe { pattern.CurrentIsSelected() }.is_ok_and(|selected| selected.as_bool())
            })
        })
    }

    fn named(&self, name: &str) -> Option<IUIAutomationElement> {
        self.tabs()
            .into_iter()
            .find(|(_, tab)| tab.trim() == name)
            .map(|(tab, _)| tab)
    }
}

fn int_variant(value: i32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_I4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { lVal: value },
            }),
        },
    }
}

fn selection(tab: &IUIAutomationElement) -> Option<IUIAutomationSelectionItemPattern> {
    unsafe { tab.GetCurrentPatternAs(UIA_SelectionItemPatternId) }.ok()
}

fn select(tab: &IUIAutomationElement) -> bool {
    selection(tab).is_some_and(|pattern| unsafe { pattern.Select() }.is_ok())
}

pub(super) fn pick(names: &[String], title: &str) -> Option<usize> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    let mut matches = names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.trim() == title)
        .map(|(index, _)| index);
    match (matches.next(), matches.next()) {
        (Some(index), None) => Some(index),
        _ => None,
    }
}

fn marker() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("qol-focus-{}-{nanos}", std::process::id())
}

fn in_apartment<T: Send>(work: impl FnOnce() -> Option<T> + Send) -> Option<T> {
    thread::scope(|scope| {
        scope
            .spawn(|| {
                let _apartment = ComApartment::enter(Apartment::MultiThreaded).ok()?;
                work()
            })
            .join()
            .ok()
            .flatten()
    })
}

pub(super) fn select_tab(window: u32, title: &str, mark: impl Fn(&str) -> bool + Send) -> bool {
    in_apartment(move || {
        let strip = TabStrip::open(window)?;
        let tabs = strip.tabs();
        if tabs.len() < 2 {
            return Some(false);
        }
        let names: Vec<String> = tabs.iter().map(|(_, name)| name.clone()).collect();
        if let Some(index) = pick(&names, title) {
            return Some(select(&tabs[index].0));
        }
        let marker = marker();
        if !mark(&marker) {
            return Some(false);
        }
        let deadline = Instant::now() + MARK_TIMEOUT;
        let found = loop {
            if let Some(tab) = strip.named(&marker) {
                break Some(tab);
            }
            if Instant::now() >= deadline {
                break None;
            }
            thread::sleep(MARK_POLL);
        };
        let selected = found.as_ref().is_some_and(select);
        mark(title);
        Some(selected)
    })
    .unwrap_or(false)
}

pub(super) fn keeping_selection<T: Send>(window: u32, work: impl FnOnce() -> T + Send) -> T {
    let outcome = thread::scope(|scope| {
        scope
            .spawn(|| {
                let apartment = ComApartment::enter(Apartment::MultiThreaded).ok();
                let strip = apartment.as_ref().and_then(|_| TabStrip::open(window));
                let selected = strip.as_ref().and_then(TabStrip::selected);
                let result = work();
                if let Some(tab) = &selected {
                    select(tab);
                }
                result
            })
            .join()
    });
    outcome.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_is_picked_only_by_one_exact_title() {
        let names: Vec<String> = ["pwsh", " Claude Code ", "lane-1", "lane-1", ""]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let cases = [
            ("Claude Code", Some(1)),
            ("pwsh ", Some(0)),
            ("lane-1", None),
            ("Claude", None),
            ("", None),
            ("   ", None),
        ];
        for (title, expected) in cases {
            assert_eq!(pick(&names, title), expected, "{title:?}");
        }
    }

    #[test]
    fn markers_are_unique_per_call() {
        let first = marker();
        std::thread::sleep(Duration::from_millis(1));
        assert_ne!(first, marker());
        assert!(first.starts_with("qol-focus-"));
    }
}
