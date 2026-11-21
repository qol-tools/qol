pub mod names;

use anyhow::Context;

use crate::device::{self, OUTPUT};

pub use crate::device::{
    DeviceRow as OutputRow, DeviceStatus as OutputStatus, StatusState, SYSTEM_DEFAULT,
};

pub fn list() -> anyhow::Result<Vec<OutputRow>> {
    device::list(OUTPUT)
}

pub fn switch(output: &str) -> anyhow::Result<()> {
    device::switch(OUTPUT, output).map(|_| ())
}

pub struct NextOutput {
    pub row: OutputRow,
    pub switched: bool,
}

pub fn next() -> anyhow::Result<NextOutput> {
    let rows = list()?;
    let effective = device::effective(OUTPUT)?;
    let index = next_index(&rows, effective.as_ref().map(|identity| identity.as_str()))
        .context("no sound outputs are connected")?;
    let row = rows[index].clone();
    if rows.len() == 1 {
        return Ok(NextOutput {
            row,
            switched: false,
        });
    }
    switch(&row.value)?;
    Ok(NextOutput {
        row,
        switched: true,
    })
}

fn next_index(rows: &[OutputRow], effective: Option<&str>) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    for (index, row) in rows.iter().enumerate() {
        if Some(row.value.as_str()) == effective {
            return Some((index + 1) % rows.len());
        }
    }
    Some(0)
}

pub fn status(saved: String) -> anyhow::Result<OutputStatus> {
    device::status(OUTPUT, saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(value: &str) -> OutputRow {
        OutputRow {
            value: value.to_string(),
            label: value.to_string(),
            picture: "speaker-default".to_string(),
            connected: true,
        }
    }

    #[test]
    fn the_next_output_index_wraps_and_falls_back_to_the_first() {
        let rows = vec![row("speaker-a"), row("speaker-b"), row("speaker-c")];
        assert_eq!(next_index(&rows, Some("speaker-a")), Some(1));
        assert_eq!(next_index(&rows, Some("speaker-b")), Some(2));
        assert_eq!(next_index(&rows, Some("speaker-c")), Some(0));
        assert_eq!(next_index(&rows, Some("speaker-x")), Some(0));
        assert_eq!(next_index(&rows, None), Some(0));
    }

    #[test]
    fn the_next_output_index_is_none_without_outputs_and_stays_on_a_single_output() {
        assert_eq!(next_index(&[], None), None);
        let rows = vec![row("speaker-a")];
        assert_eq!(next_index(&rows, Some("speaker-a")), Some(0));
        assert_eq!(next_index(&rows, Some("speaker-x")), Some(0));
    }
}
