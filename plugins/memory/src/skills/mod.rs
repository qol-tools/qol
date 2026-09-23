use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::Digest;

const STEM_MAP: [(&str, &str); 17] = [
    ("built", "build"),
    ("building", "build"),
    ("fixed", "fix"),
    ("fixing", "fix"),
    ("formatt", "format"),
    ("wrote", "write"),
    ("writing", "write"),
    ("ran", "run"),
    ("running", "run"),
    ("done", "do"),
    ("does", "do"),
    ("made", "make"),
    ("making", "make"),
    ("used", "use"),
    ("using", "use"),
    ("tested", "test"),
    ("testing", "test"),
];

#[derive(serde::Deserialize)]
pub struct SkillsIndex {
    pub schema: u32,
    #[serde(default)]
    pub walked_at: Option<f64>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub repo: Option<Repo>,
    #[serde(default)]
    pub skills: Vec<SkillMeta>,
}

#[derive(serde::Deserialize)]
pub struct Repo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub dirty: Option<bool>,
}

#[derive(serde::Deserialize)]
pub struct SkillMeta {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub title: String,
    pub rel: String,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub sections: Vec<Section>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(serde::Deserialize)]
pub struct Section {
    pub h: String,
    #[serde(default)]
    pub lead: String,
}

pub enum Freshness {
    NotIndexed,
    Unavailable,
    Stale,
    Fresh,
}

impl Freshness {
    pub fn as_str(&self) -> &'static str {
        match self {
            Freshness::NotIndexed => "not-indexed",
            Freshness::Unavailable => "unavailable",
            Freshness::Stale => "stale",
            Freshness::Fresh => "fresh",
        }
    }
}

pub fn load_index(path: &Path) -> Option<SkillsIndex> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn pool_tokens(text: &str) -> Vec<String> {
    crate::text::tokens(text)
        .into_iter()
        .map(|t| {
            STEM_MAP
                .iter()
                .find(|(k, _)| *k == t.as_str())
                .map(|(_, v)| (*v).to_string())
                .unwrap_or(t)
        })
        .collect()
}

pub fn build_meta_doc(skill: &SkillMeta) -> String {
    let headers = skill
        .sections
        .iter()
        .map(|s| s.h.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let leads = skill
        .sections
        .iter()
        .map(|s| s.lead.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let aliases = if skill.aliases.is_empty() {
        String::new()
    } else {
        skill.aliases.join(" ")
    };
    format!(
        "{} {} {} {} {} {}",
        skill.name, skill.title, skill.description, headers, leads, aliases
    )
}

pub fn probe_fresh(index: &SkillsIndex, root: &Path) -> Freshness {
    let Some(walked_at) = index.walked_at else {
        return Freshness::NotIndexed;
    };
    if !root.exists() {
        return Freshness::Unavailable;
    }
    let mut changed = 0usize;
    for s in &index.skills {
        match fs::metadata(root.join(&s.rel)) {
            Err(_) => changed += 1,
            Ok(m) => {
                if m.modified().is_ok_and(|t| mtime_ms(t) > walked_at) {
                    changed += 1;
                }
            }
        }
    }
    if changed > 0 {
        Freshness::Stale
    } else {
        Freshness::Fresh
    }
}

pub struct SplitSection {
    pub h: String,
    pub text: String,
}

pub fn split_sections(raw: &str) -> Vec<SplitSection> {
    let mut sections = Vec::new();
    let mut in_fence = false;
    for line in raw.split('\n') {
        if line.trim().starts_with("```") {
            in_fence = !in_fence;
        }
        if !in_fence && line.starts_with("## ") {
            sections.push(SplitSection {
                h: line[3..].trim().to_string(),
                text: String::new(),
            });
        } else if let Some(cur) = sections.last_mut() {
            if !line.starts_with("# ") {
                cur.text.push_str(line);
                cur.text.push('\n');
            }
        }
    }
    sections
}

pub struct BestSection {
    pub h: String,
    pub text: String,
    pub score: f64,
}

pub fn best_section(
    skill: &SkillMeta,
    root: &Path,
    qtokens: &[String],
    idf: &HashMap<String, f64>,
    cap: usize,
) -> Option<BestSection> {
    let raw = fs::read_to_string(root.join(&skill.rel)).ok()?;
    let sections = split_sections(&raw);
    let weights: Vec<f64> = qtokens
        .iter()
        .map(|t| idf.get(t).filter(|w| **w != 0.0).copied().unwrap_or(1.0))
        .collect();
    let mut best: Option<usize> = None;
    let mut best_score = 0.0f64;
    for (si, s) in sections.iter().enumerate() {
        let content_set: HashSet<String> = pool_tokens(&crate::text::utf16_slice(&s.text, 0, cap))
            .into_iter()
            .collect();
        let header_set: HashSet<String> = pool_tokens(&s.h).into_iter().collect();
        let mut acc = 0.0f64;
        for (i, t) in qtokens.iter().enumerate() {
            let in_content = content_set.contains(t);
            let in_header = header_set.contains(t);
            acc += ((in_content as i32 * 2 + in_header as i32 * 3) as f64) * weights[i];
        }
        let score = acc * if si == 0 { 0.5 } else { 1.0 };
        if score > best_score {
            best_score = score;
            best = Some(si);
        }
    }
    match best {
        Some(i) if best_score > 0.0 => Some(BestSection {
            h: sections[i].h.clone(),
            text: sections[i].text.clone(),
            score: best_score,
        }),
        _ => None,
    }
}

pub enum Served {
    Ok {
        content: String,
        section: String,
        truncated: bool,
        hash_match: bool,
        live_hash: String,
    },
    Failed {
        reason: String,
    },
}

pub fn serve_section(
    skill: &SkillMeta,
    root: &Path,
    header_hint: Option<&str>,
    cap: usize,
) -> Served {
    let p = root.join(&skill.rel);
    if !p.exists() {
        return Served::Failed {
            reason: "missing".into(),
        };
    }
    let raw = match fs::read_to_string(&p) {
        Ok(r) => r,
        Err(e) => {
            return Served::Failed {
                reason: format!("read-error: {}", e),
            }
        }
    };
    let live_hash = sha256_hex16(&raw);
    let sections = split_sections(&raw);
    let norm = |s: &str| s.to_lowercase().replace('`', "").trim().to_string();
    let mut target: Option<&SplitSection> = None;
    if let Some(hint) = header_hint {
        let hint_norm = norm(hint);
        target = sections.iter().find(|s| norm(&s.h) == hint_norm);
    }
    if target.is_none() {
        target = sections.iter().find(|s| {
            let len = crate::text::utf16_len(&s.text);
            (24..=400).contains(&len)
        });
    }
    let Some(target) = target else {
        return Served::Failed {
            reason: "no-anchor".into(),
        };
    };
    let trimmed = target.text.trim();
    Served::Ok {
        content: crate::text::utf16_slice(trimmed, 0, cap),
        section: target.h.clone(),
        truncated: crate::text::utf16_len(trimmed) > cap,
        hash_match: live_hash == skill.hash,
        live_hash,
    }
}

fn sha256_hex16(text: &str) -> String {
    let digest = sha2::Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

fn mtime_ms(st: SystemTime) -> f64 {
    st.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const FIXTURE: &str = "---\nname: demo-skill\ndescription: demo fixture body\n---\n\n# Demo Skill\n\nPreamble prose lives before any heading.\n\n## Alpha Section\nalpha beta gamma notes here.\n\n## Delta Part\ndelta beta filler words.\n\n## Code Fence\nbefore the fence.\n\n```rust\n## not a header\nalpha hidden inside fence\n```\n\nafter fence tail line.\n\n## Emoji Tail\nemoji X-glyph anchor filler.\n";
    const LIVE_HASH: &str = "d6e1bc511374e615";
    const REL_A: &str = "plugins/demo-skills/skills/alpha/SKILL.md";
    const REL_B: &str = "plugins/demo-skills/skills/beta/SKILL.md";

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "qol-memory-skills-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_rel(root: &Path, rel: &str, content: &str) -> PathBuf {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
        p
    }

    fn skill_meta(rel: &str, hash: &str) -> SkillMeta {
        SkillMeta {
            id: format!("demo-skills/{rel}"),
            name: "demo-skill".into(),
            description: "demo fixture body".into(),
            title: "Demo Skill".into(),
            rel: rel.into(),
            hash: hash.into(),
            bytes: FIXTURE.len() as u64,
            sections: vec![],
            aliases: vec![],
        }
    }

    fn index_with(walked_at: Option<f64>, rels: &[&str]) -> SkillsIndex {
        SkillsIndex {
            schema: 1,
            walked_at,
            root: None,
            repo: None,
            skills: rels.iter().map(|r| skill_meta(r, "")).collect(),
        }
    }

    #[test]
    fn split_sections_shapes_fixture_and_skips_fenced_headers() {
        let got = split_sections(FIXTURE);
        let flat: Vec<(&str, &str)> = got
            .iter()
            .map(|s| (s.h.as_str(), s.text.as_str()))
            .collect();
        assert_eq!(
            flat,
            vec![
                ("Alpha Section", "alpha beta gamma notes here.\n\n"),
                ("Delta Part", "delta beta filler words.\n\n"),
                (
                    "Code Fence",
                    "before the fence.\n\n```rust\n## not a header\nalpha hidden inside fence\n```\n\nafter fence tail line.\n\n"
                ),
                ("Emoji Tail", "emoji X-glyph anchor filler.\n\n"),
            ]
        );
        assert!(got[2].text.contains("## not a header"));
        assert!(got.iter().all(|s| !s.text.contains("Preamble")));
    }

    #[test]
    fn pool_tokens_maps_stems_verbatim_after_tokenizing() {
        assert_eq!(
            pool_tokens("BUILT ran config FIXED"),
            vec![
                "build".to_string(),
                "run".to_string(),
                "config".to_string(),
                "fix".to_string()
            ]
        );
        assert_eq!(
            pool_tokens("using making DONE formatt"),
            vec![
                "use".to_string(),
                "make".to_string(),
                "do".to_string(),
                "format".to_string()
            ]
        );
    }

    #[test]
    fn build_meta_doc_concatenates_exactly_like_the_js_template() {
        let mut sk = skill_meta(REL_A, "");
        sk.name = "qol-demo".into();
        sk.title = "Demo Title".into();
        sk.description = "Does things".into();
        sk.sections = vec![
            Section {
                h: "Alpha".into(),
                lead: "First lead.".into(),
            },
            Section {
                h: "Beta".into(),
                lead: "Second\nlead".into(),
            },
        ];
        sk.aliases = vec![];
        assert_eq!(
            build_meta_doc(&sk),
            "qol-demo Demo Title Does things Alpha Beta First lead. Second\nlead "
        );
        sk.aliases = vec!["one".into(), "two".into()];
        assert_eq!(
            build_meta_doc(&sk),
            "qol-demo Demo Title Does things Alpha Beta First lead. Second\nlead one two"
        );
    }

    #[test]
    fn load_index_parses_live_shape_and_rejects_missing_or_garbage() {
        let dir = temp_root("load-index");
        let path = dir.join("index.json");
        fs::write(
            &path,
            r#"{"schema":1,"walked_at":1786724808424,"root":"/tmp/x","repo":{"name":"qol-skills","head":"abc","dirty":false},"skills":[{"id":"p/s","name":"s","description":"d","title":"T","rel":"plugins/p/skills/s/SKILL.md","hash":"0123456789abcdef","bytes":10,"sections":[{"h":"H","lead":"L"}],"aliases":["a"]}]}"#,
        )
        .unwrap();
        let idx = load_index(&path).expect("parses");
        assert_eq!(idx.schema, 1);
        assert_eq!(idx.walked_at, Some(1786724808424.0));
        assert_eq!(idx.root.as_deref(), Some("/tmp/x"));
        assert_eq!(
            idx.repo.as_ref().and_then(|r| r.head.clone()).as_deref(),
            Some("abc")
        );
        assert_eq!(idx.skills[0].rel, "plugins/p/skills/s/SKILL.md");
        assert_eq!(idx.skills[0].sections[0].h, "H");
        assert_eq!(idx.skills[0].aliases, vec!["a".to_string()]);
        assert!(load_index(&dir.join("absent.json")).is_none());
        fs::write(dir.join("bad.json"), "{oops").unwrap();
        assert!(load_index(&dir.join("bad.json")).is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn probe_fresh_flags_touched_skills_and_all_states() {
        let root = temp_root("probe");
        let a = write_rel(&root, REL_A, FIXTURE);
        let b = write_rel(&root, REL_B, FIXTURE);
        let idx = index_with(None, &[REL_A, REL_B]);
        assert_eq!(probe_fresh(&idx, &root).as_str(), "not-indexed");

        let walked_at = mtime_ms(fs::metadata(&b).unwrap().modified().unwrap());
        let idx = index_with(Some(walked_at), &[REL_A, REL_B]);
        assert_eq!(probe_fresh(&idx, &root).as_str(), "fresh");

        let mut spin = 0;
        while mtime_ms(fs::metadata(&a).unwrap().modified().unwrap()) <= walked_at && spin < 200 {
            fs::write(&a, FIXTURE).unwrap();
            spin += 1;
        }
        assert!(
            spin < 200,
            "filesystem mtime granularity too coarse after {spin} spins"
        );
        assert_eq!(probe_fresh(&idx, &root).as_str(), "stale");

        fs::remove_file(&b).unwrap();
        let only_b = index_with(Some(walked_at), &[REL_B]);
        assert_eq!(probe_fresh(&only_b, &root).as_str(), "stale");

        let absent_root =
            std::env::temp_dir().join(format!("qol-memory-skills-absent-{}", std::process::id()));
        let _ = fs::remove_dir_all(&absent_root);
        assert_eq!(probe_fresh(&idx, &absent_root).as_str(), "unavailable");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn best_section_scores_match_lib_skills_pool_numbers() {
        let root = temp_root("best");
        write_rel(&root, REL_A, FIXTURE);
        let sk = skill_meta(REL_A, "");
        let idf = HashMap::from([
            ("alpha".to_string(), 2.0f64),
            ("beta".to_string(), 1.5f64),
            ("gamma".to_string(), 0.0f64),
            ("filler".to_string(), 0.25f64),
        ]);
        let qt: Vec<String> = ["alpha", "beta", "gamma", "filler", "missingtok"]
            .iter()
            .map(|t| t.to_string())
            .collect();

        let best = best_section(&sk, &root, &qt, &idf, 2048).expect("scored winner");
        assert_eq!(best.h, "Alpha Section");
        assert_eq!(best.text, "alpha beta gamma notes here.\n\n");
        assert_eq!(best.score, 7.5);

        assert!(best_section(&sk, &root, &["zzz".to_string()], &idf, 2048).is_none());

        let zero_idf = HashMap::from([("alpha".to_string(), 0.0f64)]);
        let zero_qt = vec!["alpha".to_string()];
        assert_eq!(
            best_section(&sk, &root, &zero_qt, &zero_idf, 2048)
                .unwrap()
                .score,
            2.5
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn serve_section_hint_hash_truncation_and_fallback_match_js() {
        let root = temp_root("serve-ok");
        write_rel(&root, REL_B, FIXTURE);
        let self_hash = skill_meta(REL_B, LIVE_HASH);

        let Served::Ok {
            content,
            section,
            truncated,
            hash_match,
            live_hash,
        } = serve_section(&self_hash, &root, Some("alpha section"), 2048)
        else {
            panic!("hint serving must succeed");
        };
        assert_eq!(section, "Alpha Section");
        assert_eq!(content, "alpha beta gamma notes here.");
        assert!(!truncated);
        assert!(hash_match);
        assert_eq!(live_hash, LIVE_HASH);

        let mismatch = skill_meta(REL_B, "0000000000000000");
        let Served::Ok {
            hash_match: false, ..
        } = serve_section(&mismatch, &root, Some("nope"), 4096)
        else {
            panic!("wrong hash must yield hash_match false");
        };

        let small_cap = skill_meta(REL_B, LIVE_HASH);
        let Served::Ok {
            content,
            section,
            truncated,
            hash_match: _,
            live_hash: _,
        } = serve_section(&small_cap, &root, Some("Delta Part"), 10)
        else {
            panic!("cap serving must succeed");
        };
        assert_eq!(content, "delta beta");
        assert_eq!(section, "Delta Part");
        assert!(truncated);

        let Served::Ok {
            section, truncated, ..
        } = serve_section(&self_hash, &root, None, 4096)
        else {
            panic!("fallback serving must succeed");
        };
        assert_eq!(section, "Alpha Section");
        assert!(!truncated);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn serve_section_reports_missing_read_error_and_no_anchor() {
        let root = temp_root("serve-fail");
        write_rel(&root, REL_B, FIXTURE);
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::write(
            root.join("empty/SKILL.md"),
            "# Title\n\ntiny.\n\n## Too Short\nabc\n",
        )
        .unwrap();

        let gone = skill_meta("gone/deep/SKILL.md", "");
        let Served::Failed { reason } = serve_section(&gone, &root, None, 2048) else {
            panic!("missing path must fail");
        };
        assert_eq!(reason, "missing");

        let dir_rel = skill_meta("empty", "");
        let Served::Failed { reason } = serve_section(&dir_rel, &root, None, 2048) else {
            panic!("directory read must fail");
        };
        assert!(reason.starts_with("read-error: "), "got: {reason}");

        let short = skill_meta("empty/SKILL.md", "");
        let Served::Failed { reason } = serve_section(&short, &root, Some("nothing here"), 2048)
        else {
            panic!("no anchor must fail");
        };
        assert_eq!(reason, "no-anchor");

        fs::remove_dir_all(&root).ok();
    }
}
