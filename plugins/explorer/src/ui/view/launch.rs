use std::io;
use std::path::{Path, PathBuf};
use std::thread;

use futures::channel::oneshot;
use gpui::{point, px, AsyncApp, ClipboardItem, Context, WeakEntity};
use qol_apps::desktop_integration::{self, OpenHandler};

use super::menu::Menu;
use super::ExplorerView;
use crate::ui::PLUGIN_ID;

pub(super) struct OpenWith {
    paths: Vec<PathBuf>,
    handlers: Vec<OpenHandler>,
}

type Found = io::Result<(Vec<PathBuf>, Vec<OpenHandler>)>;

fn same_type_as<F>(cursor: &Path, picks: Vec<PathBuf>, kind: F) -> Vec<PathBuf>
where
    F: Fn(&Path) -> Option<String>,
{
    if !picks.iter().any(|pick| pick == cursor) {
        return vec![cursor.to_path_buf()];
    }
    let wanted = kind(cursor);
    picks
        .into_iter()
        .filter(|pick| pick == cursor || (wanted.is_some() && kind(pick) == wanted))
        .collect()
}

fn copied_text(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn display(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), crate::listing::display_name)
}

impl ExplorerView {
    pub(super) fn copy_path(&mut self, cx: &mut Context<Self>) -> bool {
        let paths = self.browser.launch_paths();
        if paths.is_empty() {
            return false;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(copied_text(&paths)));
        let note = if paths.len() == 1 {
            "Copied the path".to_string()
        } else {
            format!("Copied {} paths", paths.len())
        };
        self.browser.announce(note);
        true
    }

    pub(super) fn open_terminal_here(&mut self) -> bool {
        let folder = self.browser.folder().to_path_buf();
        if let Err(error) = desktop_integration::open_terminal_in(&folder) {
            log::warn!(
                "[{PLUGIN_ID}] could not open a terminal in {}: {error}",
                folder.display()
            );
            self.browser
                .fail(format!("Could not open a terminal: {error}"));
        }
        true
    }

    pub(super) fn open_with(&mut self, cx: &mut Context<Self>) -> bool {
        if !desktop_integration::open_with_supported() {
            self.browser
                .fail("Choosing an app is not available on this system".to_string());
            return true;
        }
        let Some(cursor) = self.browser.cursor_file() else {
            return false;
        };
        let picks = self.browser.launch_paths();
        let (sent, found) = oneshot::channel::<Found>();
        let worker = cursor.clone();
        let spawned = thread::Builder::new()
            .name("qol-explorer-open-with".into())
            .spawn(move || {
                let kind = |path: &Path| desktop_integration::file_type(path).ok();
                let outcome = desktop_integration::open_handlers(&worker)
                    .map(|handlers| (same_type_as(&worker, picks, kind), handlers));
                let _ = sent.send(outcome);
            });
        if let Err(error) = spawned {
            self.could_not_choose(&cursor, &error);
            return true;
        }
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(outcome) = found.await else {
                return;
            };
            let _ = view.update(cx, |view, cx| {
                match outcome {
                    Ok((paths, handlers)) if !handlers.is_empty() => {
                        view.show_open_with(paths, handlers)
                    }
                    Ok(_) => view
                        .browser
                        .fail(format!("No app handles {}", display(&cursor))),
                    Err(error) => view.could_not_choose(&cursor, &error),
                }
                cx.notify();
            });
        })
        .detach();
        true
    }

    fn show_open_with(&mut self, paths: Vec<PathBuf>, handlers: Vec<OpenHandler>) {
        let at = point(px(self.rail_width + 24.0), px(96.0));
        self.menu = Some(Menu::open_with(&handlers, at));
        self.open_with = Some(OpenWith { paths, handlers });
    }

    fn could_not_choose(&mut self, path: &Path, error: &io::Error) {
        log::warn!(
            "[{PLUGIN_ID}] could not list apps for {}: {error}",
            path.display()
        );
        self.browser
            .fail(format!("Could not list apps for {}", display(path)));
    }

    pub(super) fn open_with_choice(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(chosen) = self.open_with.take() else {
            return;
        };
        let Some(handler) = chosen.handlers.get(index).cloned() else {
            return;
        };
        let paths = chosen.paths;
        let (sent, outcome) = oneshot::channel();
        let worker = paths.clone();
        let spawned = thread::Builder::new()
            .name("qol-explorer-open-with".into())
            .spawn(move || {
                let _ = sent.send(desktop_integration::open_with_handler(&handler.id, &worker));
            });
        if let Err(error) = spawned {
            self.could_not_open_with(&paths, &error);
            return;
        }
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Err(error)) = outcome.await else {
                return;
            };
            let _ = view.update(cx, |view, cx| {
                view.could_not_open_with(&paths, &error);
                cx.notify();
            });
        })
        .detach();
    }

    fn could_not_open_with(&mut self, paths: &[PathBuf], error: &io::Error) {
        log::warn!("[{PLUGIN_ID}] could not open with the chosen app: {error}");
        let name = paths.first().map(|path| display(path)).unwrap_or_default();
        self.browser.fail(format!("Could not open {name}: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(path: &Path) -> Option<String> {
        match path.extension()?.to_str()? {
            "txt" | "md" => Some("text/plain".into()),
            "png" => Some("image/png".into()),
            _ => None,
        }
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn open_with_covers_only_the_picks_of_the_cursor_files_type() {
        let cases = [
            ("/a.txt", &["/b.png"][..], &["/a.txt"][..]),
            (
                "/a.txt",
                &["/a.txt", "/b.png", "/c.md"],
                &["/a.txt", "/c.md"],
            ),
            ("/a.txt", &["/a.txt", "/b.png"], &["/a.txt"]),
            ("/x.bin", &["/x.bin", "/y.bin"], &["/x.bin"]),
            ("/a.txt", &[], &["/a.txt"]),
        ];
        for (cursor, picks, expected) in cases {
            let found = same_type_as(Path::new(cursor), paths(picks), kind);
            assert_eq!(found, paths(expected), "{cursor} among {picks:?}");
        }
    }

    #[test]
    fn copied_paths_are_one_per_line() {
        assert_eq!(copied_text(&paths(&["/a/b.txt"])), "/a/b.txt");
        assert_eq!(copied_text(&paths(&["/a", "/b c"])), "/a\n/b c");
    }
}
