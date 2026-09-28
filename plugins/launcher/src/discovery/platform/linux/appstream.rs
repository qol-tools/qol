use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use roxmltree::{Document, Node, ParsingOptions};

const FOLDERS: [&str; 2] = ["metainfo", "appdata"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Component {
    pub summary: Option<String>,
    pub long: Option<String>,
    pub website: Option<String>,
    pub developer: Option<String>,
    pub licence: Option<String>,
}

pub(super) fn component(desktop_id: &str) -> Option<Component> {
    static INDEX: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    let path = INDEX
        .get_or_init(|| index(&super::icons::data_dirs()))
        .get(desktop_id)?;
    read(&fs::read_to_string(path).ok()?)
}

fn index(data_dirs: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut found = HashMap::new();
    for folder in data_dirs
        .iter()
        .flat_map(|dir| FOLDERS.map(|name| dir.join(name)))
    {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.extension().is_none_or(|extension| extension != "xml") {
                continue;
            }
            for id in desktop_ids(&path) {
                found.entry(id).or_insert_with(|| path.clone());
            }
        }
    }
    found
}

fn desktop_ids(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(document) = parse(&text) else {
        return Vec::new();
    };
    document
        .root_element()
        .children()
        .filter(|node| {
            node.has_tag_name("id")
                || (node.has_tag_name("launchable") && node.attribute("type") == Some("desktop-id"))
        })
        .filter_map(|node| node.text().map(str::trim))
        .filter(|id| !id.is_empty())
        .map(|id| {
            if id.ends_with(".desktop") {
                id.to_owned()
            } else {
                format!("{id}.desktop")
            }
        })
        .collect()
}

fn parse(text: &str) -> Result<Document<'_>, roxmltree::Error> {
    Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: true,
            ..ParsingOptions::default()
        },
    )
}

fn read(text: &str) -> Option<Component> {
    let document = parse(text).ok()?;
    let root = document.root_element();
    let child = |name: &str| {
        root.children()
            .find(|node| node.has_tag_name(name) && untranslated(node))
    };
    let developer = child("developer")
        .and_then(|developer| {
            developer
                .children()
                .find(|node| node.has_tag_name("name") && untranslated(node))
        })
        .or_else(|| child("developer_name"))
        .or_else(|| child("project_group"))
        .and_then(|node| words(&node));
    let website = root
        .children()
        .find(|node| node.has_tag_name("url") && node.attribute("type") == Some("homepage"))
        .and_then(|node| words(&node));
    let long = child("description").and_then(|description| {
        description
            .children()
            .find(|node| node.has_tag_name("p") && untranslated(node))
            .map_or_else(|| words(&description), |paragraph| words(&paragraph))
    });
    Some(Component {
        summary: child("summary").and_then(|node| words(&node)),
        long,
        website,
        developer,
        licence: child("project_license").and_then(|node| words(&node)),
    })
}

fn untranslated(node: &Node<'_, '_>) -> bool {
    node.attributes()
        .all(|attribute| attribute.name() != "lang")
}

fn words(node: &Node<'_, '_>) -> Option<String> {
    let text: Vec<&str> = node
        .descendants()
        .filter(Node::is_text)
        .filter_map(|text| text.text())
        .flat_map(str::split_whitespace)
        .collect();
    (!text.is_empty()).then(|| text.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const XED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
 <id>org.x.editor</id>
 <project_license>GPL-2.0+</project_license>
 <name>Xed</name>
 <summary>A Text Editor</summary>
 <summary xml:lang="de">Ein Texteditor</summary>
 <description><p> Xed is a small,
 but powerful text editor &amp; more. </p> <p>Plugins too.</p></description>
 <description xml:lang="de"><p>Xed ist klein.</p></description>
 <url type="bugtracker">https://github.com/linuxmint/xed/issues</url>
 <url type="homepage">http://www.github.com/linuxmint/xed</url>
 <developer id="org.linuxmint"><name>Linux Mint</name></developer>
</component>"#;

    #[test]
    fn components_read_the_untranslated_facts() {
        let component = read(XED).expect("parses");
        assert_eq!(component.summary.as_deref(), Some("A Text Editor"));
        assert_eq!(
            component.long.as_deref(),
            Some("Xed is a small, but powerful text editor & more.")
        );
        assert_eq!(
            component.website.as_deref(),
            Some("http://www.github.com/linuxmint/xed")
        );
        assert_eq!(component.developer.as_deref(), Some("Linux Mint"));
        assert_eq!(component.licence.as_deref(), Some("GPL-2.0+"));
        let grouped = read("<component><project_group>GNOME</project_group></component>");
        assert_eq!(
            grouped.and_then(|component| component.developer).as_deref(),
            Some("GNOME")
        );
    }

    #[test]
    fn the_index_maps_ids_and_launchables_to_desktop_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let metainfo = dir.path().join("metainfo");
        fs::create_dir(&metainfo).unwrap();
        fs::write(metainfo.join("org.x.editor.metainfo.xml"), XED).unwrap();
        fs::write(
            metainfo.join("terminal.appdata.xml"),
            r#"<component><id>org.gnome.Terminal</id><launchable type="desktop-id">org.gnome.Terminal.desktop</launchable><developer_name>The GNOME Project</developer_name></component>"#,
        )
        .unwrap();
        let found = index(&[dir.path().to_path_buf()]);
        assert!(found.contains_key("org.x.editor.desktop"));
        let terminal = read(&fs::read_to_string(&found["org.gnome.Terminal.desktop"]).unwrap());
        assert_eq!(
            terminal
                .and_then(|component| component.developer)
                .as_deref(),
            Some("The GNOME Project")
        );
    }
}
