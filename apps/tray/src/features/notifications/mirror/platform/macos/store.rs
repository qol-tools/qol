use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const SQLITE: &str = "/usr/bin/sqlite3";
const DB_RELATIVE: &str = "Library/Group Containers/group.com.apple.usernoted/db2/db";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Cursor {
    delivered: f64,
    rec_id: i64,
}

#[derive(Debug, PartialEq)]
pub(super) struct Row {
    pub(super) cursor: Cursor,
    pub(super) data: Vec<u8>,
}

// `delivered_date` is selected as `printf('%.17g', ..)` text: the sqlite3 shell prints a
// REAL with 15 significant digits, which does not round-trip the stored value.
#[derive(Deserialize)]
struct RawCursor {
    rec_id: i64,
    delivered_date: String,
}

#[derive(Deserialize)]
struct RawRow {
    rec_id: i64,
    delivered_date: String,
    data: String,
}

fn parse_delivered(text: &str) -> Result<f64> {
    text.parse()
        .with_context(|| format!("bad delivered_date {text:?}"))
}

pub(super) fn db_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(DB_RELATIVE))
}

pub(super) fn latest(db: &Path) -> Result<Cursor> {
    let rows: Vec<RawCursor> = query(
        db,
        "select rec_id, printf('%.17g', delivered_date) as delivered_date from record \
         where delivered_date is not null \
         order by delivered_date desc, rec_id desc limit 1",
    )?;
    match rows.first() {
        Some(row) => Ok(Cursor {
            delivered: parse_delivered(&row.delivered_date)?,
            rec_id: row.rec_id,
        }),
        None => Ok(Cursor {
            delivered: 0.0,
            rec_id: 0,
        }),
    }
}

/// Rows with undecodable data are logged and returned with empty data, so the caller still
/// advances its cursor past them.
pub(super) fn since(db: &Path, cursor: Cursor) -> Result<Vec<Row>> {
    let rows: Vec<RawRow> = query(db, &since_sql(cursor))?;
    rows.into_iter()
        .map(|raw| {
            let cursor = Cursor {
                delivered: parse_delivered(&raw.delivered_date)?,
                rec_id: raw.rec_id,
            };
            let data = decode_hex(&raw.data).unwrap_or_else(|error| {
                log::warn!(
                    "notification mirror: skipping record {} with malformed hex data: {error:#}",
                    raw.rec_id
                );
                Vec::new()
            });
            Ok(Row { cursor, data })
        })
        .collect()
}

fn since_sql(cursor: Cursor) -> String {
    format!(
        "select rec_id, printf('%.17g', delivered_date) as delivered_date, \
         hex(data) as data from record \
         where delivered_date > {d} or (delivered_date = {d} and rec_id > {id}) \
         order by delivered_date, rec_id",
        d = cursor.delivered,
        id = cursor.rec_id,
    )
}

fn query<T: serde::de::DeserializeOwned>(db: &Path, sql: &str) -> Result<Vec<T>> {
    let output = Command::new(SQLITE)
        .arg("-readonly")
        .arg("-json")
        .arg(db)
        .arg(sql)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run {SQLITE}"))?;
    if !output.status.success() {
        bail!(
            "{SQLITE} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_json(&String::from_utf8_lossy(&output.stdout))
}

fn parse_json<T: serde::de::DeserializeOwned>(stdout: &str) -> Result<Vec<T>> {
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(stdout).context("sqlite3 printed malformed JSON")
}

fn decode_hex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        bail!("odd hex length {}", text.len());
    }
    (0..text.len())
        .step_by(2)
        .map(|at| {
            let pair = text.get(at..at + 2).context("non-ASCII hex")?;
            u8::from_str_radix(pair, 16).with_context(|| format!("bad hex byte {pair:?}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_sql_round_trips_the_cursor_exactly() {
        let delivered = 813_503_938.508_244_f64;
        let sql = since_sql(Cursor {
            delivered,
            rec_id: 506,
        });
        let printed = sql
            .split("delivered_date > ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap();
        assert_eq!(printed.parse::<f64>().unwrap(), delivered);
        assert!(sql.contains("rec_id > 506"));
    }

    #[test]
    fn rows_parse_from_sqlite_json() {
        let stdout = r#"[{"rec_id":7,"delivered_date":"813503659.25","data":"62706C"},
{"rec_id":8,"delivered_date":"813503938.50824404","data":"zz"}]"#;
        let rows = parse_json::<RawRow>(stdout).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            parse_delivered(&rows[1].delivered_date).unwrap(),
            813_503_938.508_244_04
        );
        assert_eq!(decode_hex(&rows[0].data).unwrap(), b"bpl");
        assert!(decode_hex(&rows[1].data).is_err());
    }

    #[test]
    fn queries_print_the_date_with_full_precision() {
        let sql = since_sql(Cursor {
            delivered: 1.0,
            rec_id: 1,
        });
        assert!(sql.contains("printf('%.17g', delivered_date) as delivered_date"));
    }

    #[test]
    fn empty_output_means_no_rows() {
        assert!(parse_json::<RawRow>("").unwrap().is_empty());
        assert!(parse_json::<RawRow>("\n").unwrap().is_empty());
    }

    #[test]
    fn decode_hex_cases() {
        let cases: [(&str, Option<&[u8]>); 5] = [
            ("", Some(&[])),
            ("00ff7F", Some(&[0x00, 0xff, 0x7f])),
            ("abc", None),
            ("zz", None),
            ("é0", None),
        ];
        for (input, want) in cases {
            assert_eq!(decode_hex(input).ok().as_deref(), want, "input {input:?}");
        }
    }
}
