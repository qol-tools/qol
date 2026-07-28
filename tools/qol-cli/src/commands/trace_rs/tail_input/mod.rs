use super::*;

mod platform;

pub(super) fn detail_control_hint(replay: bool, details: bool) -> &'static str {
    if replay {
        return if details {
            "details enabled"
        } else {
            "use --details to expand"
        };
    }
    if std::io::stdin().is_terminal() {
        "press d to toggle, ctrl+c exits"
    } else if details {
        "details enabled"
    } else {
        "use --details to expand"
    }
}

pub(super) struct DetailToggleInput {
    pub(super) enabled: bool,
    pub(super) pipe_flag: Option<Arc<AtomicBool>>,
    _guard: Option<platform::CbreakGuard>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TailControl {
    Continue,
    Exit,
}

impl DetailToggleInput {
    pub(super) fn new() -> Self {
        if !std::io::stdin().is_terminal() {
            return Self::piped();
        }
        let guard = platform::CbreakGuard::new();
        let enabled = guard.is_some();
        Self {
            enabled,
            pipe_flag: None,
            _guard: guard,
        }
    }

    pub(super) fn piped() -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        let writer = Arc::clone(&flag);
        std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            let mut stdin = std::io::stdin().lock();
            while let Ok(read) = stdin.read(&mut byte) {
                if read == 0 {
                    break;
                }
                if matches!(byte[0], b'd' | b'D') {
                    writer.store(true, Ordering::SeqCst);
                }
            }
        });
        Self {
            enabled: false,
            pipe_flag: Some(flag),
            _guard: None,
        }
    }

    pub(super) fn poll(&mut self, runner: &mut TraceRunner) -> TailControl {
        if let Some(flag) = self.pipe_flag.as_ref() {
            if flag.swap(false, Ordering::SeqCst) {
                println!("{}\n", runner.toggle_details());
                runner.flush();
            }
            return TailControl::Continue;
        }
        if !self.enabled {
            return TailControl::Continue;
        }
        let has_event = match event::poll(Duration::ZERO) {
            Ok(has_event) => has_event,
            Err(_) => {
                self.enabled = false;
                return TailControl::Continue;
            }
        };
        if !has_event {
            return TailControl::Continue;
        }
        let Ok(TerminalEvent::Key(key)) = event::read() else {
            return TailControl::Continue;
        };
        Self::handle_key(runner, key.code, key.modifiers, key.kind)
    }

    pub(super) fn handle_key(
        runner: &mut TraceRunner,
        code: KeyCode,
        modifiers: KeyModifiers,
        kind: KeyEventKind,
    ) -> TailControl {
        if kind != KeyEventKind::Press {
            return TailControl::Continue;
        }
        if is_ctrl_c(code, modifiers) {
            return TailControl::Exit;
        }
        if matches!(code, KeyCode::Char('d') | KeyCode::Char('D')) {
            println!("{}\n", runner.toggle_details());
            runner.flush();
        }
        TailControl::Continue
    }
}

pub(super) fn is_ctrl_c(code: KeyCode, modifiers: KeyModifiers) -> bool {
    match code {
        KeyCode::Char('c') | KeyCode::Char('C') => modifiers.contains(KeyModifiers::CONTROL),
        KeyCode::Char('\u{3}') => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_input_maps_detail_toggle_and_ctrl_c() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));

        let toggle = DetailToggleInput::handle_key(
            &mut runner,
            KeyCode::Char('d'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(toggle, TailControl::Continue);
        assert!(runner.args.details);

        let release = DetailToggleInput::handle_key(
            &mut runner,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        );
        assert_eq!(release, TailControl::Continue);

        let exit = DetailToggleInput::handle_key(
            &mut runner,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        );
        assert_eq!(exit, TailControl::Exit);
    }
}
