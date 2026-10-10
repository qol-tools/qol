use std::io::{self, Read};
use std::time::Duration;

use windows_sys::Win32::System::Console::{
    GenerateConsoleCtrlEvent, GetConsoleMode, WriteConsoleInputW, CTRL_C_EVENT,
    ENABLE_PROCESSED_INPUT, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT, KEY_EVENT_RECORD,
    KEY_EVENT_RECORD_0,
};

use super::attach::{ConsoleFile, Detached};
use super::keys::{text_strokes, Key, KeyStroke};

pub(super) const INSERT: &str = "insert";
pub(super) const SUBMIT: &str = "submit";
pub(super) const KEY: &str = "key";

const SUBMIT_DELAY: Duration = Duration::from_millis(50);
const WRITE_BATCH: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Request {
    Text { pid: u32, submit: bool },
    Key { pid: u32, key: Key },
}

impl Request {
    fn pid(&self) -> u32 {
        match self {
            Request::Text { pid, .. } | Request::Key { pid, .. } => *pid,
        }
    }
}

fn parse(args: &[String]) -> io::Result<Request> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
    let (pid, rest) = args
        .split_first()
        .ok_or_else(|| invalid("expected a process id".to_owned()))?;
    let pid = pid
        .parse::<u32>()
        .map_err(|_| invalid(format!("invalid process id {pid:?}")))?;
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    match rest.as_slice() {
        [INSERT] => Ok(Request::Text { pid, submit: false }),
        [SUBMIT] => Ok(Request::Text { pid, submit: true }),
        [KEY, name] => Key::parse(name)
            .map(|key| Request::Key { pid, key })
            .ok_or_else(|| invalid(format!("unsupported key {name:?}"))),
        _ => Err(invalid(format!(
            "expected {INSERT}, {SUBMIT} or {KEY} NAME after the process id"
        ))),
    }
}

pub(super) fn run(args: &[String]) -> io::Result<String> {
    let request = parse(args)?;
    let mut text = String::new();
    if matches!(request, Request::Text { .. }) {
        io::stdin().read_to_string(&mut text)?;
    }
    let scope = Detached::begin();
    let _attached = scope.attach(request.pid())?;
    let input = ConsoleFile::open("CONIN$")?;
    let written = match request {
        Request::Text { submit, .. } => {
            let mut written = write(&input, &text_strokes(&text))?;
            if submit {
                std::thread::sleep(SUBMIT_DELAY);
                written += write(&input, &Key::Enter.strokes())?;
            }
            written
        }
        Request::Key {
            key: Key::CtrlC, ..
        } if processed_input(&input) => {
            if unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) } == 0 {
                return Err(io::Error::last_os_error());
            }
            0
        }
        Request::Key { key, .. } => write(&input, &key.strokes())?,
    };
    Ok(format!("wrote {written} key events"))
}

fn processed_input(input: &ConsoleFile) -> bool {
    let mut mode = 0u32;
    let read = unsafe { GetConsoleMode(input.0, &mut mode) };
    read != 0 && mode & ENABLE_PROCESSED_INPUT != 0
}

fn record(stroke: &KeyStroke) -> INPUT_RECORD {
    INPUT_RECORD {
        EventType: KEY_EVENT as u16,
        Event: INPUT_RECORD_0 {
            KeyEvent: KEY_EVENT_RECORD {
                bKeyDown: i32::from(stroke.down),
                wRepeatCount: 1,
                wVirtualKeyCode: stroke.virtual_key,
                wVirtualScanCode: stroke.scan_code,
                uChar: KEY_EVENT_RECORD_0 {
                    UnicodeChar: stroke.unit,
                },
                dwControlKeyState: stroke.control,
            },
        },
    }
}

fn write(input: &ConsoleFile, strokes: &[KeyStroke]) -> io::Result<usize> {
    let records: Vec<INPUT_RECORD> = strokes.iter().map(record).collect();
    let mut offset = 0;
    while offset < records.len() {
        let batch = &records[offset..records.len().min(offset + WRITE_BATCH)];
        let mut written = 0u32;
        let ok = unsafe {
            WriteConsoleInputW(input.0, batch.as_ptr(), batch.len() as u32, &mut written)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "the console accepted no input events",
            ));
        }
        offset += written as usize;
    }
    Ok(records.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_arguments_name_the_process_and_the_delivery() {
        let cases: [(&[&str], Option<Request>); 9] = [
            (
                &["42", "insert"],
                Some(Request::Text {
                    pid: 42,
                    submit: false,
                }),
            ),
            (
                &["42", "submit"],
                Some(Request::Text {
                    pid: 42,
                    submit: true,
                }),
            ),
            (
                &["7", "key", "esc"],
                Some(Request::Key {
                    pid: 7,
                    key: Key::Escape,
                }),
            ),
            (
                &["7", "key", "ctrl+c"],
                Some(Request::Key {
                    pid: 7,
                    key: Key::CtrlC,
                }),
            ),
            (&["7", "key", "f13"], None),
            (&["7", "key"], None),
            (&["-1", "insert"], None),
            (&["42"], None),
            (&[], None),
        ];
        for (args, expected) in cases {
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            assert_eq!(parse(&args).ok(), expected, "{args:?}");
        }
    }

    #[test]
    fn records_are_key_events_with_one_repeat() {
        for stroke in text_strokes("a").into_iter().chain(Key::CtrlC.strokes()) {
            let record = record(&stroke);
            assert_eq!(u32::from(record.EventType), KEY_EVENT);
            let key = unsafe { record.Event.KeyEvent };
            assert_eq!(key.bKeyDown != 0, stroke.down);
            assert_eq!(key.wRepeatCount, 1);
            assert_eq!(key.wVirtualKeyCode, stroke.virtual_key);
            assert_eq!(key.wVirtualScanCode, stroke.scan_code);
            assert_eq!(unsafe { key.uChar.UnicodeChar }, stroke.unit);
            assert_eq!(key.dwControlKeyState, stroke.control);
        }
    }
}
