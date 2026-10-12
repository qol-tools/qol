use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const SQLITE: &str = "/usr/bin/sqlite3";
const DB_RELATIVE: &str = "Library/Group Containers/group.com.apple.usernoted/db2/db";
const END_QUERY: &str = "select 'qol-end' as qol_end;";
const END_LINE: &str = r#"[{"qol_end":"qol-end"}]"#;

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

pub(super) struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
}

impl Session {
    pub(super) fn open(db: &Path) -> Result<Self> {
        let mut child = Command::new(SQLITE)
            .arg("-readonly")
            .arg("-bail")
            .arg("-json")
            .arg(db)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to start {SQLITE}"))?;
        let stdin = child.stdin.take().context("sqlite3 stdin missing")?;
        let stdout = child.stdout.take().context("sqlite3 stdout missing")?;
        let stderr = child.stderr.take().context("sqlite3 stderr missing")?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr,
        })
    }

    pub(super) fn since(&mut self, cursor: Cursor) -> Result<Vec<Row>> {
        let reply = writeln!(self.stdin, "{}\n{END_QUERY}", since_sql(cursor))
            .and_then(|()| self.stdin.flush())
            .context("sqlite3 stopped accepting queries")
            .and_then(|()| read_reply(&mut self.stdout));
        match reply {
            Ok(reply) => Ok(rows_from(parse_json(&reply)?)),
            Err(error) => Err(self.with_stderr(error)),
        }
    }

    /// `-bail` makes sqlite3 exit on a failed statement, so its stderr is complete here.
    fn with_stderr(&mut self, error: anyhow::Error) -> anyhow::Error {
        let _ = self.child.kill();
        let mut message = String::new();
        let _ = self.stderr.read_to_string(&mut message);
        match message.trim() {
            "" => error,
            message => error.context(message.to_string()),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_reply(reader: &mut impl BufRead) -> Result<String> {
    let mut reply = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader
            .read_line(&mut line)
            .context("sqlite3 output unreadable")?
            == 0
        {
            bail!("sqlite3 exited mid-reply");
        }
        if line.trim_end() == END_LINE {
            return Ok(reply);
        }
        reply.push_str(&line);
    }
}

fn rows_from(rows: Vec<RawRow>) -> Vec<Row> {
    rows.into_iter()
        .filter_map(|raw| {
            let delivered = parse_delivered(&raw.delivered_date)
                .inspect_err(|error| log::warn!("notification mirror: {error:#}"))
                .ok()?;
            let data = decode_hex(&raw.data).unwrap_or_else(|error| {
                log::warn!(
                    "notification mirror: skipping record {} with malformed hex data: {error:#}",
                    raw.rec_id
                );
                Vec::new()
            });
            Some(Row {
                cursor: Cursor {
                    delivered,
                    rec_id: raw.rec_id,
                },
                data,
            })
        })
        .collect()
}

fn since_sql(cursor: Cursor) -> String {
    format!(
        "select rec_id, printf('%.17g', delivered_date) as delivered_date, \
         hex(data) as data from record \
         where delivered_date > {d} or (delivered_date = {d} and rec_id > {id}) \
         order by delivered_date, rec_id;",
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
    fn rows_parse_from_sqlite_json_and_skip_bad_data() {
        let stdout = r#"[{"rec_id":7,"delivered_date":"813503659.25","data":"62706C"},
{"rec_id":8,"delivered_date":"813503938.5","data":"zz"},
{"rec_id":9,"delivered_date":"not a date","data":""}]"#;
        let rows = rows_from(parse_json::<RawRow>(stdout).unwrap());
        assert_eq!(
            rows,
            vec![
                Row {
                    cursor: Cursor {
                        delivered: 813_503_659.25,
                        rec_id: 7
                    },
                    data: b"bpl".to_vec(),
                },
                Row {
                    cursor: Cursor {
                        delivered: 813_503_938.5,
                        rec_id: 8
                    },
                    data: Vec::new(),
                },
            ]
        );
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
    fn read_reply_stops_at_the_end_marker() {
        let output = format!("[{{\"rec_id\":7}},\n{{\"rec_id\":8}}]\n{END_LINE}\n{END_LINE}\n");
        let mut reader = std::io::Cursor::new(output);
        assert_eq!(
            read_reply(&mut reader).unwrap(),
            "[{\"rec_id\":7},\n{\"rec_id\":8}]\n"
        );
        assert_eq!(read_reply(&mut reader).unwrap(), "");
        assert!(read_reply(&mut reader).is_err());
    }

    #[test]
    fn session_reads_the_end_marker_sqlite_prints() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db");
        let created = Command::new(SQLITE)
            .arg(&db)
            .arg(
                "create table record (rec_id integer primary key, delivered_date real, data blob); \
                 insert into record values (1, 10.5, x'0102'), (2, 11.25, x'ff');",
            )
            .status()
            .unwrap();
        assert!(created.success());
        let mut session = Session::open(&db).unwrap();
        let first = session
            .since(Cursor {
                delivered: 10.5,
                rec_id: 1,
            })
            .unwrap();
        assert_eq!(
            first,
            vec![Row {
                cursor: Cursor {
                    delivered: 11.25,
                    rec_id: 2
                },
                data: vec![0xff],
            }]
        );
        let none = session
            .since(Cursor {
                delivered: 11.25,
                rec_id: 2,
            })
            .unwrap();
        assert!(none.is_empty());
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
