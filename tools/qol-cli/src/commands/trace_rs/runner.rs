use super::*;

pub(super) struct TraceRunner {
    pub(super) args: Args,
    pub(super) path: PathBuf,
    pub(super) pid_names: HashMap<String, String>,
    pub(super) buffer: Vec<Event>,
    pub(super) last_event_at: Option<Instant>,
    pub(super) last_pick: Option<(String, String, String, String, String)>,
    pub(super) monitors: Vec<(i64, i64, i64, i64)>,
    pub(super) last_summary: Option<String>,
    pub(super) picker_statuses: HashMap<String, String>,
    pub(super) winact_fail_pids: HashSet<String>,
    pub(super) pending_activation: Option<PendingActivation>,
    pub(super) target_opacities: HashMap<String, f64>,
    pub(super) opacity_state: HashMap<String, OpacityWrite>,
    pub(super) ghost_dump_active: bool,
    pub(super) dumped_windows: Vec<GhostWindow>,
    pub(super) stats: TraceStats,
    pub(super) opacity_waste: OpacityWaste,
}

impl TraceRunner {
    pub(super) fn new(args: Args, path: PathBuf) -> Self {
        Self {
            args,
            path,
            pid_names: HashMap::new(),
            buffer: Vec::new(),
            last_event_at: None,
            last_pick: None,
            monitors: Vec::new(),
            last_summary: None,
            picker_statuses: HashMap::new(),
            winact_fail_pids: HashSet::new(),
            pending_activation: None,
            target_opacities: HashMap::new(),
            opacity_state: HashMap::new(),
            ghost_dump_active: false,
            dumped_windows: Vec::new(),
            stats: TraceStats::default(),
            opacity_waste: OpacityWaste::default(),
        }
    }

    pub(super) fn run(&mut self) -> Result<()> {
        if !self.args.no_header {
            self.print_header();
        }
        self.query_initial_monitors();
        let start_ts = self.start_ts();
        let file = File::open(&self.path)
            .with_context(|| format!("failed to open trace log {}", self.path.display()))?;
        let mut reader = BufReader::new(file);

        if self.args.replay {
            println!("{COLOR_DIM}Replaying full log...{COLOR_RESET}\n");
            self.replay(&mut reader, start_ts)?;
            self.flush();
            self.print_stats();
            self.print_opacity_waste();
            return Ok(());
        }

        if start_ts.is_none() {
            reader.seek(SeekFrom::End(0))?;
        }
        self.tail(reader, start_ts)
    }

    pub(super) fn print_header(&self) {
        let mode = if self.args.details {
            "expanded"
        } else {
            "collapsed"
        };
        match self.args.plugin.as_deref() {
            Some(plugin) => println!(
                "{COLOR_HEADER}Tailing {} filtering for {COLOR_OK}{plugin}{COLOR_HEADER}...{COLOR_RESET}",
                self.path.display()
            ),
            None => println!(
                "{COLOR_HEADER}Tailing {} (system runtime trace)...{COLOR_RESET}",
                self.path.display()
            ),
        }
        println!(
            "{COLOR_DIM}Aggregating transitions into {mode} trace groups ({}).{COLOR_RESET}\n",
            detail_control_hint(self.args.replay, self.args.details)
        );
    }

    pub(super) fn start_ts(&self) -> Option<u64> {
        let since = self.args.since?;
        Some(now_ms().saturating_sub(since.as_millis() as u64))
    }

    pub(super) fn replay(
        &mut self,
        reader: &mut BufReader<File>,
        start_ts: Option<u64>,
    ) -> Result<()> {
        let mut prev_ts = None;
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            let Some(raw) = parse_raw_line(line.trim_end()) else {
                continue;
            };
            if start_ts.is_some_and(|start| raw.ts_ms < start) {
                continue;
            }
            if prev_ts.is_some_and(|prev| raw.ts_ms.saturating_sub(prev) > REPLAY_GAP_MS) {
                self.flush();
            }
            prev_ts = Some(raw.ts_ms);
            self.process_raw(raw);
        }
        Ok(())
    }

    pub(super) fn tail(
        &mut self,
        mut reader: BufReader<File>,
        start_ts: Option<u64>,
    ) -> Result<()> {
        let mut detail_input = DetailToggleInput::new();
        let mut line = String::new();
        loop {
            if detail_input.poll(self) == TailControl::Exit {
                self.finish_tail();
                return Ok(());
            }
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                if self
                    .last_event_at
                    .is_some_and(|at| at.elapsed() > TAIL_FLUSH_AFTER)
                {
                    self.flush();
                }
                std::thread::sleep(TAIL_IDLE_SLEEP);
                continue;
            }
            let Some(raw) = parse_raw_line(line.trim_end()) else {
                continue;
            };
            if start_ts.is_some_and(|start| raw.ts_ms < start) {
                continue;
            }
            self.process_raw(raw);
        }
    }

    pub(super) fn finish_tail(&mut self) {
        self.flush();
        self.print_stats();
        self.print_opacity_waste();
        println!("\n{COLOR_HEADER}Exiting tailer.{COLOR_RESET}");
    }

    pub(super) fn toggle_details(&mut self) -> String {
        self.args.details = !self.args.details;
        let state = if self.args.details {
            "expanded"
        } else {
            "collapsed"
        };
        format!("{COLOR_HEADER}Trace details: {state}{COLOR_RESET}")
    }

    pub(super) fn process_raw(&mut self, raw: RawLine) {
        self.resolve_pending_activation(raw.ts_ms);
        self.confirm_pending_activation(&raw);

        if !self.args.topic.matches(&raw.tag) {
            return;
        }
        if self.args.no_ghosts && matches!(raw.tag.as_str(), "GHOSTDUMP" | "GHOSTWIN" | "SUMMARY") {
            return;
        }
        if self.args.no_opacity && matches!(raw.tag.as_str(), "HIDE_WIN" | "SHOW_WIN") {
            return;
        }
        self.register_monitors(&raw.msg);

        let Some(event) = self.format_event(raw) else {
            return;
        };
        self.push_event(event);
    }

    pub(super) fn push_event(&mut self, mut event: Event) {
        if let Some(plugin) = self.args.plugin.as_deref() {
            let publish_mentions_plugin = event.tag == "PUBLISH" && event.text.contains(plugin);
            let filter_source = event.filter_source.as_deref().unwrap_or(&event.source);
            if filter_source != plugin && !publish_mentions_plugin {
                return;
            }
        }
        if let Some(grep) = self.args.grep.as_deref() {
            if !event.text.to_lowercase().contains(&grep.to_lowercase()) {
                return;
            }
        }
        if self.args.anomalies
            && !ANOMALY_MARKERS
                .iter()
                .any(|marker| event.text.contains(marker))
        {
            return;
        }
        if event.tag == "SUMMARY" && self.last_summary.as_ref() == Some(&event.text) {
            return;
        }
        if event.tag == "SUMMARY" {
            self.last_summary = Some(event.text.clone());
        }

        event.ts = format_timestamp(event.ts_ms);
        self.buffer.push(event);
        self.last_event_at = Some(Instant::now());
    }

    pub(super) fn resolve_pending_activation(&mut self, now_ms: u64) {
        let Some(pending) = self.pending_activation.as_ref() else {
            return;
        };
        if now_ms.saturating_sub(pending.ts_ms) <= 600 {
            return;
        }
        let pending = self.pending_activation.take().expect("pending activation");
        let ts_ms = pending.ts_ms.saturating_add(600);
        let (tag, text) = if pending.confirmed_front {
            self.stats.focus_ok += 1;
            (
                "FOCUS",
                format!(
                    "{COLOR_OK}✔ FOCUS OK{COLOR_RESET}: \"{}\" (wid: {}) confirmed front; no WM focus-change event.",
                    pending.title, pending.wid
                ),
            )
        } else {
            self.stats.focus_timeout += 1;
            (
                "FOCUS_WARN",
                format!(
                    "{COLOR_FAIL}✖ FOCUS FAILURE{COLOR_RESET}: Timed out focusing \"{}\" (wid: {}). WM ignored request.",
                    pending.title, pending.wid
                ),
            )
        };
        self.push_event(Event {
            ts_ms,
            ts: String::new(),
            tag: tag.to_string(),
            source: "host".to_string(),
            filter_source: Some(pending.source),
            text,
        });
    }

    pub(super) fn confirm_pending_activation(&mut self, raw: &RawLine) {
        if !matches!(raw.tag.as_str(), "ACTIVATE_SETTLED" | "ACTIVATE_KEY_FOCUS") {
            return;
        }
        let Some(pending) = self.pending_activation.as_mut() else {
            return;
        };
        if field(&raw.msg, "wid") == Some(pending.wid.as_str()) {
            pending.confirmed_front = true;
        }
    }

    pub(super) fn format_event(&mut self, raw: RawLine) -> Option<Event> {
        let source = self.source_for(&raw);
        let text = match raw.tag.as_str() {
            "PICK" => self.format_pick(&raw)?,
            "AMC" => {
                let title = field(&raw.msg, "active_visible")?;
                format!(
                    "Target -> {COLOR_AMC}{}{COLOR_RESET}",
                    self.compact_title(title)
                )
            }
            "HOST_EMIT_AMC" => {
                let new_idx = field(&raw.msg, "new_idx").unwrap_or("?");
                let is_boot = field(&raw.msg, "is_boot").unwrap_or("?");
                format!("HOST_EMIT_AMC: new_idx={new_idx} (is_boot={is_boot})")
            }
            "PLUGIN_RECV_AMC" => {
                let idx = field(&raw.msg, "monitor_idx").unwrap_or("?");
                format!("PLUGIN_RECV_AMC: monitor_idx={idx}")
            }
            "PUBLISH" => self
                .format_publish(&raw.msg)
                .unwrap_or_else(|| format!("PUBLISH {}", raw.msg)),
            "SUBSCRIBE" => self.format_subscribe(&raw.msg),
            "RECV" => self
                .format_recv(&raw.msg)
                .unwrap_or_else(|| format!("RECV {}", raw.msg)),
            "ALT_POLL_START" => {
                let title = field(&raw.msg, "title").unwrap_or("alt-tab");
                let title = self.compact_title(title);
                self.record_opened_popup(&title, raw.ts_ms);
                format!("Opened ({title})")
            }
            "DISMISS" => self.format_dismiss(&raw.msg),
            "CYCLE" => self.format_cycle(&raw.msg)?,
            "ACTIVATE" => self
                .format_activate(&raw, &source)
                .unwrap_or_else(|| format!("ACTIVATE: {}", raw.msg)),
            "ACTIVATE_WIN" => self
                .format_activate_win(&raw, &source)
                .unwrap_or_else(|| format!("ACTIVATE_WIN: {}", raw.msg)),
            "WM_RECEIVE" => {
                return self.format_wm_receive(&raw).or_else(|| {
                    Some(Event {
                        ts_ms: raw.ts_ms,
                        ts: String::new(),
                        tag: raw.tag,
                        source,
                        filter_source: None,
                        text: format!("WM_RECEIVE: {}", raw.msg),
                    })
                });
            }
            "FOCUS" => {
                return self.format_focus(&raw).or_else(|| {
                    Some(Event {
                        ts_ms: raw.ts_ms,
                        ts: String::new(),
                        tag: raw.tag,
                        source,
                        filter_source: None,
                        text: format!("FOCUS: {}", raw.msg),
                    })
                });
            }
            "FOCUS_WIN" => {
                return self.format_focus_win(&raw).or_else(|| {
                    Some(Event {
                        ts_ms: raw.ts_ms,
                        ts: String::new(),
                        tag: raw.tag,
                        source: "host".to_string(),
                        filter_source: None,
                        text: format!("FOCUS_WIN: {}", raw.msg),
                    })
                });
            }
            "PICKER_STALE" => self.format_picker_status(&raw.msg, "stale")?,
            "PICKER_READY" => self.format_picker_status(&raw.msg, "ready")?,
            "HIDE_WIN" => self.format_hide_win(&raw.msg, raw.ts_ms)?,
            "SHOW_WIN" => self.format_show_win(&raw.msg, raw.ts_ms)?,
            "GHOSTDUMP" => return self.format_ghostdump(&raw, &source),
            "GHOSTWIN" => {
                self.record_ghostwin(&raw.msg, raw.ts_ms);
                return None;
            }
            "GHOST_DUMP" => self.format_ghost_dump(&raw.msg)?,
            "LEGEND" => format!("LEGEND {}", raw.msg),
            tag if tag.starts_with("LAUNCHER_") => self.format_launcher_event(tag, &raw.msg),
            tag if tag.starts_with("WINACT_") => self.format_winact_event(&raw),
            tag if tag.starts_with("PROFILE_") => {
                let color = if raw.msg.contains("outcome=include")
                    && raw.msg.contains("entry_kind=symlink")
                {
                    COLOR_FAIL
                } else {
                    ""
                };
                let reset = if color.is_empty() { "" } else { COLOR_RESET };
                format!("{color}{}: {}{reset}", raw.tag, raw.msg)
            }
            tag if tag.starts_with("WORLD_") => {
                let is_bad = raw.msg.contains("outcome=reject")
                    || raw.msg.contains("outcome=skip")
                    || raw.msg.contains("reason=already_dived")
                    || raw.msg.contains("visible_slots=0");
                let color = if is_bad { COLOR_FAIL } else { "" };
                let reset = if color.is_empty() { "" } else { COLOR_RESET };
                format!("{color}{}: {}{reset}", raw.tag, raw.msg)
            }
            "MARK" => quoted_field(&raw.msg, "message")
                .map(|message| format!("MARK: {message}"))
                .unwrap_or_else(|| format!("MARK: {}", raw.msg)),
            _ => {
                let color = if raw.msg.contains("DIVERGENCE") {
                    COLOR_FAIL
                } else {
                    ""
                };
                let reset = if color.is_empty() { "" } else { COLOR_RESET };
                format!("{color}{}: {}{reset}", raw.tag, raw.msg)
            }
        };
        Some(Event {
            ts_ms: raw.ts_ms,
            ts: String::new(),
            tag: raw.tag,
            source,
            filter_source: None,
            text,
        })
    }

    pub(super) fn format_publish(&self, msg: &str) -> Option<String> {
        let idx = field(msg, "idx")?;
        let name = first_quoted(msg).unwrap_or_else(|| "?".to_string());
        let is_boot = field(msg, "is_boot").unwrap_or("?");
        let delivered = bracket_field(msg, "delivered").unwrap_or_else(|| "?".to_string());
        let missed = bracket_field(msg, "missed").unwrap_or_else(|| "?".to_string());
        Some(format!(
            "PUBLISH AMC idx={idx} \"{name}\" is_boot={is_boot} -> delivered=[{delivered}] missed=[{missed}]"
        ))
    }

    pub(super) fn format_subscribe(&self, msg: &str) -> String {
        let plugin = field(msg, "plugin").unwrap_or("?");
        let interests = bracket_field(msg, "interests").unwrap_or_else(|| "?".to_string());
        let replay = msg
            .split_once("-> host sticky-replay AMC idx=")
            .map(|(_, idx)| format!(" -> host sticky-replay AMC idx={}", idx.trim()))
            .unwrap_or_default();
        format!("SUBSCRIBE plugin={plugin} interests=[{interests}]{replay}")
    }

    pub(super) fn format_recv(&self, msg: &str) -> Option<String> {
        let idx = field(msg, "idx")?;
        let name = first_quoted(msg).unwrap_or_else(|| "?".to_string());
        let src = field(msg, "src").unwrap_or("?");
        Some(format!("RECV AMC idx={idx} \"{name}\" src={src}"))
    }

    pub(super) fn format_dismiss(&self, msg: &str) -> String {
        let src = field(msg, "from").unwrap_or("unknown");
        let title = field(msg, "title").unwrap_or("");
        let title_suffix = if title.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.compact_title(title))
        };
        let src_color =
            if src.contains("alt-up") || src.contains("super-up") || src.contains("modifiers/") {
                COLOR_OK
            } else {
                COLOR_WARN
            };
        format!("Closed{title_suffix} (from={src_color}{src}{COLOR_RESET})")
    }

    pub(super) fn format_cycle(&self, msg: &str) -> Option<String> {
        let cycle = parse_python_cycle(msg)?;
        let method = cycle.method;
        let from = cycle.from;
        let to = cycle.to;
        let count = cycle.count;
        let elapsed = cycle.elapsed_ms;
        let elapsed_ms = elapsed.parse::<u64>().unwrap_or(0);
        let target = if cycle.title.is_empty() {
            cycle.app.to_string()
        } else {
            format!("{}: {}", cycle.app, cycle.title)
        };
        Some(format!(
            "Cycle {COLOR_HOTKEY}{method}{COLOR_RESET} [{from}->{to}/{count}] -> {target} {}({elapsed}ms){COLOR_RESET}",
            latency_color(elapsed_ms),
        ))
    }

    pub(super) fn format_activate(&mut self, raw: &RawLine, event_source: &str) -> Option<String> {
        if self
            .args
            .plugin
            .as_deref()
            .is_some_and(|plugin| plugin != event_source)
        {
            return None;
        }
        let seq = sequence(&raw.msg)?.to_string();
        let wid = field(&raw.msg, "wid")?.to_string();
        let title = quoted_field(&raw.msg, "title")?;
        let source = field(&raw.msg, "source").unwrap_or("?");
        let sent_ts = field(&raw.msg, "sent_ts").unwrap_or("?");
        let requestor_active = field(&raw.msg, "requestor_active").unwrap_or("?");
        self.stats.focus_req += 1;
        self.pending_activation = Some(PendingActivation {
            ts_ms: raw.ts_ms,
            seq: Some(seq.clone()),
            wid: wid.clone(),
            title: title.clone(),
            source: event_source.to_string(),
            confirmed_front: false,
        });
        Some(format!(
            "➔ ACTIVATE #{seq}: focus \"{title}\" (wid: {wid}) source={source} sent_ts={sent_ts} requestor_active={requestor_active}"
        ))
    }

    pub(super) fn format_activate_win(
        &mut self,
        raw: &RawLine,
        event_source: &str,
    ) -> Option<String> {
        if self
            .args
            .plugin
            .as_deref()
            .is_some_and(|plugin| plugin != event_source)
        {
            return None;
        }
        let wid = field(&raw.msg, "wid")?.to_string();
        let title = quoted_field(&raw.msg, "title")?;
        if let Some(previous) = self.pending_activation.as_ref() {
            self.stats.supersede += 1;
            let text = format!(
                "{COLOR_WARN}⚠ SUPERSEDED{COLOR_RESET}: New request to focus \"{title}\" arrived before focus on \"{}\" was confirmed.",
                previous.title
            );
            self.push_event(Event {
                ts_ms: raw.ts_ms,
                ts: String::new(),
                tag: "FOCUS_WARN".to_string(),
                source: event_source.to_string(),
                filter_source: None,
                text,
            });
        }
        self.stats.focus_req += 1;
        self.pending_activation = Some(PendingActivation {
            ts_ms: raw.ts_ms,
            seq: None,
            wid: wid.clone(),
            title: title.clone(),
            source: event_source.to_string(),
            confirmed_front: false,
        });
        let payload = ewmh_payload(&raw.msg);
        Some(format!(
            "➔ ACTIVATE REQUEST: focus \"{title}\" (wid: {wid}){payload}"
        ))
    }

    pub(super) fn format_wm_receive(&self, raw: &RawLine) -> Option<Event> {
        let seq = sequence(&raw.msg)?;
        let target = quoted_field(&raw.msg, "target")?;
        let user_time = field(&raw.msg, "_NET_WM_USER_TIME").unwrap_or("?");
        let msg_ts = field(&raw.msg, "msg_ts").unwrap_or("?");
        let (comparison, status) =
            arrow_status(&raw.msg).unwrap_or(("?".to_string(), "?".to_string()));
        let filter_source = self
            .pending_activation
            .as_ref()
            .filter(|pending| pending.seq.as_deref() == Some(seq))
            .map(|pending| pending.source.clone());
        if self.args.plugin.is_some() && filter_source.is_none() {
            return None;
        }
        Some(Event {
            ts_ms: raw.ts_ms,
            ts: String::new(),
            tag: raw.tag.clone(),
            source: "host".to_string(),
            filter_source,
            text: format!(
            "WM_RECEIVE #{seq}: target=\"{target}\" _NET_WM_USER_TIME={user_time} msg_ts={msg_ts} -> {comparison} ({status})"
            ),
        })
    }

    pub(super) fn format_focus(&mut self, raw: &RawLine) -> Option<Event> {
        let seq = sequence(&raw.msg)?;
        let result = field(&raw.msg, "result")?;
        let active_after = field(&raw.msg, "active_after").unwrap_or("?");
        let demands_attention = field(&raw.msg, "demands_attention").unwrap_or("?");
        let elapsed = field(&raw.msg, "elapsed")
            .map(|value| value.trim_end_matches("ms"))
            .unwrap_or("?");
        let color = if result == "OK" { COLOR_OK } else { COLOR_FAIL };
        let filter_source = match self
            .pending_activation
            .as_ref()
            .filter(|pending| pending.seq.as_deref() == Some(seq))
        {
            Some(pending)
                if self
                    .args
                    .plugin
                    .as_deref()
                    .is_some_and(|plugin| plugin != pending.source) =>
            {
                return None;
            }
            Some(pending) => {
                let source = pending.source.clone();
                self.pending_activation = None;
                Some(source)
            }
            None if self.args.plugin.is_some() => return None,
            None => None,
        };
        let text = if result == "OK" {
            format!(
                "{color}✔ FOCUS #{seq} SUCCESS{COLOR_RESET}: active_after={active_after} elapsed={elapsed}ms"
            )
        } else {
            format!(
                "{color}✖ FOCUS #{seq} {result}{COLOR_RESET}: active_after={active_after} demands_attention={demands_attention} elapsed={elapsed}ms"
            )
        };
        Some(Event {
            ts_ms: raw.ts_ms,
            ts: String::new(),
            tag: raw.tag.clone(),
            source: "host".to_string(),
            filter_source,
            text,
        })
    }

    pub(super) fn format_focus_win(&mut self, raw: &RawLine) -> Option<Event> {
        let (x, y) = tuple_field(&raw.msg, "winpos")?;
        let title = quoted_field(&raw.msg, "title")?;
        let focused_wid = field(&raw.msg, "wid").map(ToString::to_string);
        let ignored = if field(&raw.msg, "ignored") == Some("true") {
            format!(" {COLOR_WARN}(ignored){COLOR_RESET}")
        } else {
            String::new()
        };
        let proc_info = field(&raw.msg, "pid")
            .map(|pid| format!(" (proc: {}, pid: {pid})", self.process_name(pid)))
            .unwrap_or_default();

        if let Some(pending) = self.pending_activation.as_ref() {
            if self
                .args
                .plugin
                .as_deref()
                .is_some_and(|plugin| plugin != pending.source)
            {
                return None;
            }
        }

        if let Some(pending) = self.pending_activation.take() {
            let latency = raw.ts_ms.saturating_sub(pending.ts_ms);
            let is_match = if let Some(focused) = focused_wid.as_deref() {
                focused == pending.wid
            } else {
                title_contains_match(&title, &pending.title)
            };
            let detect = field(&raw.msg, "detect_lag_ms")
                .map(|lag| format!(" (detect ±{lag}ms)"))
                .unwrap_or_default();
            let text = if is_match {
                self.record_focus_ok(raw.ts_ms, &pending.wid, latency);
                format!(
                    "{COLOR_OK}✔ FOCUS SUCCESS{COLOR_RESET}: Focused \"{title}\" (wid: {}) in {COLOR_OK}{latency}ms{COLOR_RESET}{detect}{ignored}",
                    pending.wid
                )
            } else {
                self.stats.focus_misdirect += 1;
                format!(
                    "{COLOR_WARN}⚠ MISDIRECTED FOCUS{COLOR_RESET}: Requested \"{}\" (wid: {}), but focused \"{title}\" (wid: {}){proc_info} after {latency}ms.",
                    pending.title,
                    pending.wid,
                    focused_wid.as_deref().unwrap_or("?")
                )
            };
            return Some(Event {
                ts_ms: raw.ts_ms,
                ts: String::new(),
                tag: "FOCUS".to_string(),
                source: "host".to_string(),
                filter_source: Some(pending.source),
                text,
            });
        }

        if self.args.plugin.is_some() {
            return None;
        }
        let short_title = truncate_chars(&title, 30);
        Some(Event {
            ts_ms: raw.ts_ms,
            ts: String::new(),
            tag: "FOCUS".to_string(),
            source: "host".to_string(),
            filter_source: None,
            text: format!(
                "Active window: \"{short_title}\"{proc_info} on {}{ignored}",
                self.monitor_name(x, y)
            ),
        })
    }

    pub(super) fn format_picker_status(&mut self, msg: &str, status: &str) -> Option<String> {
        let title = field(msg, "title")?;
        let title = self.compact_title(title);
        if self
            .picker_statuses
            .get(&title)
            .is_some_and(|old| old == status)
        {
            return None;
        }
        self.picker_statuses
            .insert(title.clone(), status.to_string());
        let (label, color) = if status == "ready" {
            ("READY", COLOR_OK)
        } else {
            ("STALE", COLOR_WARN)
        };
        Some(format!("{title} -> {color}{label}{COLOR_RESET}"))
    }

    pub(super) fn format_hide_win(&mut self, msg: &str, ts_ms: u64) -> Option<String> {
        let title = self.compact_title(field(msg, "title")?);
        let (opacity, path) = hide_opacity(msg)?;
        let cls = self.record_opacity_write(&title, opacity, reason(msg), ts_ms);
        if cls == Some(OpacityClassification::Redundant) {
            return None;
        }
        let op_color = if opacity > 0.0 { COLOR_OK } else { COLOR_DIM };
        Some(format!(
            "{title} -> {op_color}{}{COLOR_RESET}{}{}{}",
            format_opacity(opacity),
            reason_suffix(msg),
            path_suffix(path),
            churn_suffix(cls.as_ref())
        ))
    }

    pub(super) fn format_show_win(&mut self, msg: &str, ts_ms: u64) -> Option<String> {
        let title = self.compact_title(field(msg, "title")?);
        let opacity = show_opacity(msg)?;
        let cls = self.record_opacity_write(&title, opacity, reason(msg), ts_ms);
        if cls == Some(OpacityClassification::Redundant) {
            return None;
        }
        Some(format!(
            "{title} -> {COLOR_OK}{}{COLOR_RESET}{}{}{}",
            format_opacity(opacity),
            reason_suffix(msg),
            ewmh_payload(msg),
            churn_suffix(cls.as_ref())
        ))
    }

    pub(super) fn format_ghost_dump(&mut self, msg: &str) -> Option<String> {
        let ghost = parse_python_ghost_dump(msg)?;
        let is_qol_window = ghost.title.starts_with("qol-");
        let level_value = ghost.level.parse::<i64>().ok();
        let alpha_value = ghost.alpha.parse::<f64>().unwrap_or(0.0);
        let alpha = format_python_float(ghost.alpha);
        let wrong_level = is_qol_window && level_value == Some(0);
        let opaque_outside_show = is_qol_window
            && alpha_value >= 1.0
            && ghost.mouse_ignored == "false"
            && !ghost.ctx.starts_with("show");
        let text = format!(
            "\"{}\" alpha={} level={} mouse_ignored={} {} ({})",
            ghost.title, alpha, ghost.level, ghost.mouse_ignored, ghost.frame, ghost.ctx
        );
        if !wrong_level && !opaque_outside_show {
            return Some(text);
        }
        let why = if wrong_level {
            "ghost at normal window level"
        } else {
            "opaque clickable ghost outside show"
        };
        self.record_divergence(why);
        Some(format!(
            "{COLOR_FAIL}⚠ DIVERGENCE ({why}): {text}{COLOR_RESET}"
        ))
    }

    pub(super) fn format_ghostdump(&mut self, raw: &RawLine, source: &str) -> Option<Event> {
        if raw.msg.contains("begin") {
            self.ghost_dump_active = true;
            self.dumped_windows.clear();
            return None;
        }
        if !raw.msg.contains("end") {
            return Some(Event {
                ts_ms: raw.ts_ms,
                ts: String::new(),
                tag: raw.tag.clone(),
                source: source.to_string(),
                filter_source: None,
                text: format!("GHOSTDUMP: {}", raw.msg),
            });
        }
        self.ghost_dump_active = false;
        let text = self.summarize_ghost_dump(raw.ts_ms);
        Some(Event {
            ts_ms: raw.ts_ms,
            ts: String::new(),
            tag: "SUMMARY".to_string(),
            source: "host".to_string(),
            filter_source: self.args.plugin.clone(),
            text,
        })
    }

    pub(super) fn record_ghostwin(&mut self, msg: &str, ts_ms: u64) {
        if !self.ghost_dump_active {
            return;
        }
        let Some(window) = parse_ghost_window(msg, ts_ms) else {
            return;
        };
        self.dumped_windows.push(window);
    }

    pub(super) fn summarize_ghost_dump(&mut self, ts_ms: u64) -> String {
        let windows = std::mem::take(&mut self.dumped_windows);
        let mut active_ghosts = Vec::new();
        let mut active_pickers = Vec::new();
        let mut inactive_visible = Vec::new();
        let mut divergence_msgs = Vec::new();
        let mut seen_titles = HashSet::new();
        let mut plugin_wins: HashMap<String, Vec<(String, f64, String)>> = HashMap::new();

        for window in windows {
            if !seen_titles.insert(window.title.clone()) {
                continue;
            }
            let comp_title = self.compact_title(&window.title);
            let proc_name = self.ghost_process_name(&window.owner_pid, &window.title);
            if self
                .args
                .plugin
                .as_deref()
                .is_some_and(|plugin| plugin != proc_name)
            {
                continue;
            }

            let map_suffix = if window.map_state == "viewable" {
                String::new()
            } else {
                format!(" ({})", window.map_state)
            };
            let proc_suffix = if proc_name == "unknown" {
                String::new()
            } else {
                format!("/{proc_name}")
            };
            let status_suffix = format!(
                "[{}]",
                self.picker_statuses
                    .get(&comp_title)
                    .map(String::as_str)
                    .unwrap_or("stale")
            );

            if window.opacity > 0.0 {
                plugin_wins.entry(proc_name.clone()).or_default().push((
                    comp_title.clone(),
                    window.opacity,
                    map_suffix.clone(),
                ));

                let label = format!(
                    "{comp_title}{proc_suffix}({}{map_suffix}){status_suffix}",
                    format_opacity(window.opacity)
                );
                match window.role.as_str() {
                    "ghost" => active_ghosts.push(label),
                    "live"
                        if window.title.contains("alt-tab")
                            || window.title.contains("launcher") =>
                    {
                        active_pickers.push(label);
                    }
                    "invisible" => inactive_visible.push(label),
                    _ => {}
                }
            }

            let expected_opacity = self
                .target_opacities
                .get(&comp_title)
                .copied()
                .unwrap_or(0.0);
            let is_actually_hidden = window.map_state != "viewable" || window.opacity <= 0.01;
            let is_expected_hidden = expected_opacity <= 0.01;
            let is_stale_sample = self
                .opacity_state
                .get(&comp_title)
                .is_some_and(|state| window.sample_ts_ms < state.ts_ms);
            let hidden_as_expected = is_actually_hidden && is_expected_hidden;
            if !is_stale_sample
                && !hidden_as_expected
                && (window.opacity - expected_opacity).abs() > 0.01
            {
                divergence_msgs.push(format!(
                    "{comp_title} opacity is {}{map_suffix}, expected {}{}",
                    format_opacity(window.opacity),
                    format_opacity(expected_opacity),
                    self.write_attribution(&comp_title, ts_ms)
                ));
            }

            if !is_actually_hidden {
                if let Some((expected_x, expected_y)) = parse_qol_title_origin(&window.title) {
                    let expected_mon = self.monitor_name_by_origin(expected_x, expected_y);
                    let actual_mon = self.monitor_name(window.x, window.y);
                    if actual_mon != expected_mon {
                        divergence_msgs.push(format!(
                            "{comp_title} is on {actual_mon} (expected {expected_mon})"
                        ));
                    }
                }
            }
        }

        for (proc_name, wins) in plugin_wins {
            if proc_name == "unknown" || wins.len() <= 1 {
                continue;
            }
            let labels = wins
                .iter()
                .map(|(title, opacity, map_suffix)| {
                    format!(
                        "{title}({}{map_suffix}){}",
                        format_opacity(*opacity),
                        self.write_attribution(title, ts_ms)
                    )
                })
                .collect::<Vec<_>>();
            inactive_visible.push(format!(
                "Multiple active {proc_name}: {}",
                labels.join(", ")
            ));
        }

        let mut active_parts = Vec::new();
        if !active_ghosts.is_empty() {
            active_parts.push(format!("Active Ghost: {}", active_ghosts.join(", ")));
        }
        if !active_pickers.is_empty() {
            active_parts.push(format!("Active Picker: {}", active_pickers.join(", ")));
        }

        let all_divergences = inactive_visible
            .into_iter()
            .chain(divergence_msgs)
            .collect::<Vec<_>>();
        let status = if all_divergences.is_empty() {
            self.stats.last_divergence = None;
            format!("{COLOR_OK}OK{COLOR_RESET}")
        } else {
            let mut sorted = all_divergences.clone();
            sorted.sort();
            self.record_divergence(&sorted.join(", "));
            format!(
                "{COLOR_FAIL}DIVERGENCE: {}{COLOR_RESET}",
                all_divergences.join(", ")
            )
        };
        let active_text = if active_parts.is_empty() {
            "No Active Win".to_string()
        } else {
            active_parts.join(" | ")
        };
        format!("{active_text} | {status}")
    }

    pub(super) fn record_focus_ok(&mut self, ts_ms: u64, wid: &str, latency_ms: u64) {
        self.stats.focus_ok += 1;
        self.stats.latencies.push(latency_ms);
        self.stats.focus_history.push((ts_ms, wid.to_string()));
        while self
            .stats
            .focus_history
            .first()
            .is_some_and(|(old_ts, _)| ts_ms.saturating_sub(*old_ts) > 2000)
        {
            self.stats.focus_history.remove(0);
        }
        let history = &self.stats.focus_history;
        if history.len() < 3 {
            return;
        }
        let last = &history[history.len() - 1].1;
        let prev = &history[history.len() - 2].1;
        let before_prev = &history[history.len() - 3].1;
        if last == before_prev && last != prev {
            self.stats.oscillation += 1;
        }
    }

    pub(super) fn record_divergence(&mut self, key: &str) {
        if self.stats.last_divergence.as_deref() == Some(key) {
            return;
        }
        self.stats.divergence += 1;
        self.stats.last_divergence = Some(key.to_string());
    }

    pub(super) fn print_stats(&self) {
        let Some(text) = self.stats_text() else {
            return;
        };
        print!("{text}");
    }

    pub(super) fn stats_text(&self) -> Option<String> {
        if !self.args.stats {
            return None;
        }
        let resolved = self.stats.focus_ok + self.stats.focus_misdirect + self.stats.focus_timeout;
        let mut out = String::new();
        let _ = writeln!(out, "\n{COLOR_HEADER}═══ SESSION STATS ═══{COLOR_RESET}");
        let _ = writeln!(out, "  Focus requests sent:  {}", self.stats.focus_req);
        let _ = writeln!(out, "  Focus resolved:       {resolved}");
        let _ = writeln!(
            out,
            "    {COLOR_OK}✔ success{COLOR_RESET}      {}",
            self.stats.focus_ok
        );
        let _ = writeln!(
            out,
            "    {COLOR_WARN}⚠ misdirected{COLOR_RESET}  {}",
            self.stats.focus_misdirect
        );
        let _ = writeln!(
            out,
            "    {COLOR_FAIL}✖ timed out{COLOR_RESET}    {}",
            self.stats.focus_timeout
        );
        let _ = writeln!(out, "    ⚡ superseded   {}", self.stats.supersede);
        let _ = writeln!(out, "    ⟳ oscillations {}", self.stats.oscillation);
        let _ = writeln!(out, "    ⚠ divergences  {}", self.stats.divergence);
        if !self.stats.latencies.is_empty() {
            let _ = writeln!(
                out,
                "  Focus latency ms: p50={} p95={} max={} min={} (n={})",
                percentile(&self.stats.latencies, 50),
                percentile(&self.stats.latencies, 95),
                self.stats.latencies.iter().max().copied().unwrap_or(0),
                self.stats.latencies.iter().min().copied().unwrap_or(0),
                self.stats.latencies.len()
            );
        }
        let _ = writeln!(out, "{COLOR_HEADER}═════════════════════{COLOR_RESET}");
        Some(out)
    }

    pub(super) fn print_opacity_waste(&self) {
        let Some(text) = self.opacity_waste_text() else {
            return;
        };
        print!("{text}");
    }

    pub(super) fn opacity_waste_text(&self) -> Option<String> {
        let total = self.opacity_waste.writes;
        if total == 0 {
            return None;
        }
        let mut out = String::new();
        let redundant_pct = 100 * self.opacity_waste.redundant / total;
        let _ = writeln!(out, "\n{COLOR_HEADER}═══ OPACITY CHURN ═══{COLOR_RESET}");
        let _ = writeln!(
            out,
            "  Opacity writes:      {total}  {COLOR_DIM}(each = popup visibility write; cached WID avoids repeat scans){COLOR_RESET}"
        );
        let _ = writeln!(
            out,
            "  {COLOR_WARN}Redundant (no-op){COLOR_RESET}:   {} ({redundant_pct}%)  {COLOR_DIM}burned round-trips{COLOR_RESET}",
            self.opacity_waste.redundant
        );
        let _ = writeln!(
            out,
            "  {COLOR_FAIL}Reverts (self-heal){COLOR_RESET}: {}",
            self.opacity_waste.reverts
        );
        if !self.opacity_waste.by_reason.is_empty() {
            let _ = writeln!(out, "  Writes by reason:");
            for (reason, count) in sorted_counts(
                &self.opacity_waste.by_reason,
                &self.opacity_waste.reason_order,
            ) {
                let red = self
                    .opacity_waste
                    .redundant_by_reason
                    .get(&reason)
                    .copied()
                    .unwrap_or(0);
                let redstr = if red == 0 {
                    String::new()
                } else {
                    format!("  {COLOR_DIM}({red} redundant){COLOR_RESET}")
                };
                let _ = writeln!(out, "    {reason:<10} {count}{redstr}");
            }
        }
        if !self.opacity_waste.revert_pairs.is_empty() {
            let _ = writeln!(
                out,
                "  {COLOR_FAIL}Self-heal pairs{COLOR_RESET} {COLOR_DIM}(firepit -> firefighter){COLOR_RESET}:"
            );
            for (pair, count) in sorted_counts(
                &self.opacity_waste.revert_pairs,
                &self.opacity_waste.revert_pair_order,
            ) {
                let _ = writeln!(out, "    {pair:<22} ×{count}");
            }
        }
        let _ = writeln!(out, "{COLOR_HEADER}═════════════════════{COLOR_RESET}");
        Some(out)
    }

    pub(super) fn ghost_process_name(&mut self, owner_pid: &str, title: &str) -> String {
        let process = if owner_pid.is_empty() {
            "unknown".to_string()
        } else {
            self.process_name(owner_pid)
        };
        if !process.chars().all(|ch| ch.is_ascii_digit()) {
            return process;
        }
        if title.contains("alt-tab") {
            return "alt-tab".to_string();
        }
        if title.contains("launcher") {
            return "launcher".to_string();
        }
        "unknown".to_string()
    }

    pub(super) fn write_attribution(&self, comp_title: &str, ts_ms: u64) -> String {
        let Some(state) = self.opacity_state.get(comp_title) else {
            return String::new();
        };
        format!(
            " ←{} {}ms ago",
            state.reason,
            ts_ms.saturating_sub(state.ts_ms)
        )
    }

    pub(super) fn format_launcher_event(&self, tag: &str, msg: &str) -> String {
        match tag {
            "LAUNCHER_SHOW" => {
                let path = field(msg, "path").unwrap_or("?");
                let title = field(msg, "title")
                    .map(|title| self.compact_title(title))
                    .unwrap_or_else(|| "?".to_string());
                if let Some((x, y, w, h)) = launcher_pos_size(msg) {
                    format!("Launcher show {COLOR_OK}{path}{COLOR_RESET} {title} {w}x{h}@({x},{y})")
                } else {
                    format!("Launcher show {COLOR_OK}{path}{COLOR_RESET} {title}")
                }
            }
            "LAUNCHER_INPUT" => {
                let key = field(msg, "key").unwrap_or("?");
                let effect = field(msg, "effect").unwrap_or("?");
                let q = quoted_field(msg, "q").unwrap_or_default();
                let selected = field(msg, "selected").unwrap_or("?");
                let results = field(msg, "results_before").unwrap_or("?");
                format!(
                    "Launcher input {COLOR_HOTKEY}{key}{COLOR_RESET} -> {effect} q=\"{q}\" selected={selected} results_before={results}"
                )
            }
            "LAUNCHER_RESIZE" => {
                let q = quoted_field(msg, "q").unwrap_or_default();
                let rows = field(msg, "rows").unwrap_or("?");
                let results = field(msg, "results").unwrap_or("?");
                let from_h = field(msg, "from_h").unwrap_or("?");
                let to_h = field(msg, "to_h").unwrap_or("?");
                format!(
                    "{COLOR_OPACITY}Launcher resize{COLOR_RESET} h {from_h}->{to_h} rows={rows} results={results} q=\"{q}\" {}",
                    launcher_window(msg)
                )
            }
            "LAUNCHER_RENDER" => {
                let q = quoted_field(msg, "q").unwrap_or_default();
                let selected_name = quoted_field(msg, "selected_name").unwrap_or_default();
                let results = field(msg, "results").unwrap_or("?");
                let visible = field(msg, "visible").unwrap_or("?");
                let selected = field(msg, "selected").unwrap_or("?");
                let scroll = field(msg, "scroll").unwrap_or("?");
                let hidden = field(msg, "hidden").unwrap_or("?");
                let target_h = field(msg, "target_h").unwrap_or("?");
                let visual_h = field(msg, "visual_h").unwrap_or("?");
                let total_us = field(msg, "total_us").unwrap_or("?");
                let filter_us = field(msg, "filter_us").unwrap_or("?");
                let rows_us = field(msg, "rows_us").unwrap_or("?");
                format!(
                    "Launcher render q=\"{q}\" results={results} visible={visible} selected={selected} \"{selected_name}\" scroll={scroll} hidden={hidden} {} target_h={target_h} visual_h={visual_h} {COLOR_DIM}time={total_us}us filter={filter_us}us rows={rows_us}us{COLOR_RESET}",
                    launcher_window(msg)
                )
            }
            "LAUNCHER_DISMISS" => {
                let src = field(msg, "from").unwrap_or("?");
                let q = quoted_field(msg, "q").unwrap_or_default();
                let results = field(msg, "results").unwrap_or("?");
                let selected = field(msg, "selected").unwrap_or("?");
                let selected_name = quoted_field(msg, "selected_name").unwrap_or_default();
                format!(
                    "Launcher closed from={COLOR_WARN}{src}{COLOR_RESET} q=\"{q}\" results={results} selected={selected} \"{selected_name}\""
                )
            }
            _ => format!("{tag}: {msg}"),
        }
    }

    pub(super) fn format_winact_event(&mut self, raw: &RawLine) -> String {
        let partial = match raw.tag.as_str() {
            "WINACT_AX" if field(&raw.msg, "outcome") == Some("fail") => {
                self.winact_fail_pids.insert(raw.pid.clone());
                false
            }
            "WINACT_DONE" => self.winact_fail_pids.remove(&raw.pid),
            _ => false,
        };

        match raw.tag.as_str() {
            "WINACT_AX" => {
                let op = field(&raw.msg, "op").unwrap_or("?");
                let pid = field(&raw.msg, "pid").unwrap_or("?");
                let dur = field(&raw.msg, "dur_ms").unwrap_or("?");
                let dur_ms = dur.parse::<u64>().unwrap_or(0);
                let outcome = field(&raw.msg, "outcome").unwrap_or("?");
                let pid_suffix = if matches!(pid, "0" | "-1" | "?") {
                    String::new()
                } else {
                    format!(" pid={pid}")
                };
                format!(
                    "  {COLOR_DIM}AX{COLOR_RESET} {op}{pid_suffix} {}{dur}ms{COLOR_RESET} {}{outcome}{COLOR_RESET}",
                    latency_color(dur_ms),
                    winact_outcome_color(outcome)
                )
            }
            "WINACT_MINIMIZE" => {
                let branch = field(&raw.msg, "branch").unwrap_or("?");
                let visible = field(&raw.msg, "visible").unwrap_or("?");
                let regular = field(&raw.msg, "regular").unwrap_or("?");
                let outcome = field(&raw.msg, "outcome").unwrap_or("?");
                let label = match branch {
                    "hide" => "hide (instant)",
                    "minimize" => "minimize (animated)",
                    "hide-fallback" => "hide fallback",
                    "minimize-fallback" => "minimize fallback",
                    other => other,
                };
                format!(
                    "  {COLOR_DIM}strategy{COLOR_RESET} {label} visible={visible} regular={regular} {}{outcome}{COLOR_RESET}",
                    winact_outcome_color(outcome)
                )
            }
            "WINACT_DONE" => {
                let action = field(&raw.msg, "action").unwrap_or("?");
                let total = field(&raw.msg, "total_ms").unwrap_or("?");
                let total_ms = total.parse::<u64>().unwrap_or(0);
                let outcome = field(&raw.msg, "outcome").unwrap_or("?");
                let verdict = if outcome == "ok" {
                    if partial {
                        format!("{COLOR_WARN}ok (partial: an AX op failed){COLOR_RESET}")
                    } else {
                        format!("{COLOR_OK}ok{COLOR_RESET}")
                    }
                } else {
                    let detail = raw
                        .msg
                        .split_once(" err=")
                        .map(|(_, err)| format!(": {err}"))
                        .unwrap_or_default();
                    format!("{COLOR_FAIL}FAILED{detail}{COLOR_RESET}")
                };
                format!(
                    "{COLOR_HOTKEY}▶ {action}{COLOR_RESET} {}{total}ms{COLOR_RESET} {verdict}",
                    latency_color(total_ms)
                )
            }
            _ => format!("{}: {}", raw.tag, raw.msg),
        }
    }

    pub(super) fn record_opened_popup(&mut self, title: &str, ts_ms: u64) {
        self.target_opacities.insert(title.to_string(), 1.0);
        self.set_opacity_state(title, 1.0, "open", ts_ms);
        let prefix = title.split('@').next().unwrap_or(title).to_string();
        let titles = self
            .target_opacities
            .keys()
            .filter(|key| key.starts_with(&prefix) && key.as_str() != title)
            .cloned()
            .collect::<Vec<_>>();
        for title in titles {
            self.target_opacities.insert(title.clone(), 0.0);
            self.set_opacity_state(&title, 0.0, "open", ts_ms);
        }
    }

    pub(super) fn record_opacity_write(
        &mut self,
        title: &str,
        opacity: f64,
        reason: &str,
        ts_ms: u64,
    ) -> Option<OpacityClassification> {
        self.target_opacities.insert(title.to_string(), opacity);
        self.opacity_waste.writes += 1;
        increment_ordered_count(
            &mut self.opacity_waste.by_reason,
            &mut self.opacity_waste.reason_order,
            reason,
        );
        let classification = self.opacity_state.get(title).and_then(|state| {
            if opacity_eq(state.op, opacity) {
                return Some(OpacityClassification::Redundant);
            }
            if state.prev_op.is_some_and(|prev| opacity_eq(prev, opacity))
                && state.reason != reason
                && ts_ms.saturating_sub(state.ts_ms) <= REVERT_WINDOW_MS
            {
                return Some(OpacityClassification::Revert {
                    previous_reason: state.reason.clone(),
                    age_ms: ts_ms.saturating_sub(state.ts_ms),
                });
            }
            None
        });
        match classification.as_ref() {
            Some(OpacityClassification::Redundant) => {
                self.opacity_waste.redundant += 1;
                increment_count(&mut self.opacity_waste.redundant_by_reason, reason);
            }
            Some(OpacityClassification::Revert {
                previous_reason, ..
            }) => {
                self.opacity_waste.reverts += 1;
                increment_ordered_count(
                    &mut self.opacity_waste.revert_pairs,
                    &mut self.opacity_waste.revert_pair_order,
                    &format!("{previous_reason}->{reason}"),
                );
            }
            None => {}
        }
        self.set_opacity_state(title, opacity, reason, ts_ms);
        classification
    }

    pub(super) fn set_opacity_state(
        &mut self,
        title: &str,
        opacity: f64,
        reason: &str,
        ts_ms: u64,
    ) {
        match self.opacity_state.get_mut(title) {
            None => {
                self.opacity_state.insert(
                    title.to_string(),
                    OpacityWrite {
                        op: opacity,
                        reason: reason.to_string(),
                        ts_ms,
                        prev_op: None,
                    },
                );
            }
            Some(state) if !opacity_eq(state.op, opacity) => {
                state.prev_op = Some(state.op);
                state.op = opacity;
                state.reason = reason.to_string();
                state.ts_ms = ts_ms;
            }
            Some(state) => {
                state.reason = reason.to_string();
                state.ts_ms = ts_ms;
            }
        }
    }

    pub(super) fn format_pick(&mut self, raw: &RawLine) -> Option<String> {
        if self.args.plugin.is_some() {
            return None;
        }
        let cursor = tuple_field(&raw.msg, "cursor")?;
        let focus = tuple_field(&raw.msg, "focus")?;
        let cursor_age = field(&raw.msg, "cursor_age_ms")?.to_string();
        let focus_age = field(&raw.msg, "focus_age_ms")?.to_string();
        let winner = field(&raw.msg, "winner")?.to_string();
        let key = (
            winner.clone(),
            cursor.0.to_string(),
            cursor.1.to_string(),
            focus.0.to_string(),
            focus.1.to_string(),
        );
        if self.last_pick.as_ref() == Some(&key) {
            return None;
        }
        self.last_pick = Some(key);

        let cursor_age_ms = cursor_age.parse::<u64>().unwrap_or(0);
        let focus_age_ms = focus_age.parse::<u64>().unwrap_or(0);
        let cursor_status = active_status(cursor_age_ms);
        let focus_status = active_status(focus_age_ms);
        let winner_color = if winner == "cursor" {
            COLOR_OK
        } else {
            COLOR_FOCUS
        };
        Some(format!(
            "Winner -> {winner_color}{}{COLOR_RESET} | Cursor: {} (age: {:.2}s {cursor_status}) | Focus: {} (age: {:.2}s {focus_status})",
            winner.to_uppercase(),
            self.monitor_name(cursor.0, cursor.1),
            cursor_age_ms as f64 / 1000.0,
            self.monitor_name(focus.0, focus.1),
            focus_age_ms as f64 / 1000.0,
        ))
    }

    pub(super) fn source_for(&mut self, raw: &RawLine) -> String {
        match raw.tag.as_str() {
            "PICK" | "AMC" | "HOST_EMIT_AMC" | "PUBLISH" | "LEGEND" | "MARK" | "WM_RECEIVE"
            | "FOCUS" => "host".to_string(),
            "SUBSCRIBE" => field(&raw.msg, "plugin")
                .map(ToString::to_string)
                .unwrap_or_else(|| self.process_name(&raw.pid)),
            tag if tag.starts_with("PROFILE_") => "profile".to_string(),
            tag if tag.starts_with("WORLD_") => "world".to_string(),
            tag if tag.starts_with("WINACT_") => "window-actions".to_string(),
            _ => self.process_name(&raw.pid),
        }
    }

    pub(super) fn process_name(&mut self, pid: &str) -> String {
        if let Some(name) = self.pid_names.get(pid) {
            return name.clone();
        }
        let name = Command::new("ps")
            .args(["-p", pid, "-o", "ucomm="])
            .output()
            .ok()
            .and_then(|output| {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                (!text.is_empty()).then_some(text)
            })
            .unwrap_or_else(|| pid.to_string());
        self.pid_names.insert(pid.to_string(), name.clone());
        name
    }

    pub(super) fn register_monitors(&mut self, msg: &str) {
        for token in msg.split_whitespace() {
            if let Some(bounds) = parse_at_bounds(token) {
                self.push_monitor(bounds);
            }
        }
        for bounds in parse_monitor_bounds_debug(msg) {
            self.push_monitor(bounds);
        }
    }

    pub(super) fn query_initial_monitors(&mut self) {
        let Some(stdout) = Command::new("xrandr")
            .arg("--current")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| output.stdout)
        else {
            return;
        };
        for line in String::from_utf8_lossy(&stdout).lines() {
            if let Some(bounds) = parse_xrandr_geometry_line(line) {
                self.push_monitor(bounds);
            }
        }
    }

    pub(super) fn push_monitor(&mut self, bounds: (i64, i64, i64, i64)) {
        if self.monitors.contains(&bounds) {
            return;
        }
        self.monitors.push(bounds);
        self.monitors.sort_by_key(|(x, y, _, _)| (*x, *y));
    }

    pub(super) fn monitor_name(&self, x: i64, y: i64) -> String {
        self.monitors
            .iter()
            .enumerate()
            .find(|(_, (mx, my, w, h))| *mx <= x && x < *mx + *w && *my <= y && y < *my + *h)
            .map(|(idx, _)| format!("Mon {idx}"))
            .unwrap_or_else(|| format!("({x},{y})"))
    }

    pub(super) fn monitor_name_by_origin(&self, x: i64, y: i64) -> String {
        self.monitors
            .iter()
            .enumerate()
            .find(|(_, (mx, my, _, _))| *mx == x && *my == y)
            .map(|(idx, _)| format!("Mon {idx}"))
            .unwrap_or_else(|| format!("({x},{y})"))
    }

    pub(super) fn compact_title(&self, title: &str) -> String {
        let Some((prefix, rest)) = title.split_once('@') else {
            return title.to_string();
        };
        if !prefix.starts_with("qol-") {
            return title.to_string();
        }
        let mut parts = rest.split(',');
        let Some(x) = parts.next().and_then(|value| value.parse::<i64>().ok()) else {
            return title.to_string();
        };
        let Some(y) = parts.next().and_then(|value| value.parse::<i64>().ok()) else {
            return title.to_string();
        };
        format!("{prefix}@{}", self.monitor_name_by_origin(x, y))
    }

    pub(super) fn flush(&mut self) {
        let Some(group) = self.flush_text() else {
            return;
        };
        println!("{group}");
    }

    pub(super) fn flush_text(&mut self) -> Option<String> {
        let unique = self.drain_unique_events();
        if unique.is_empty() {
            return None;
        }
        self.last_event_at = None;
        Some(format_group(&unique, self.args.details))
    }

    pub(super) fn drain_unique_events(&mut self) -> Vec<Event> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        let mut seen = HashSet::new();
        let mut unique = Vec::new();
        for event in self.buffer.drain(..) {
            if !event.tag.starts_with("WINACT_") && !seen.insert(event.text.clone()) {
                continue;
            }
            unique.push(event);
        }
        unique
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_events_are_deduped_until_winner_or_positions_change() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        let raw = RawLine {
            ts_ms: 1,
            pid: "1".to_string(),
            tag: "PICK".to_string(),
            msg: "cursor=(0,0) cursor_age_ms=1 focus=(0,0) focus_age_ms=2 winner=cursor"
                .to_string(),
        };
        assert!(runner.format_pick(&raw).is_some());
        assert!(runner.format_pick(&raw).is_none());
    }

    #[test]
    fn register_monitors_compacts_debug_bounds_titles() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        runner.register_monitors(
            "target=MonitorBounds { x: 0.0, y: -40.0, width: 1800.0, height: 1169.0 }",
        );
        assert_eq!(
            runner.compact_title("qol-alt-tab-picker@0,-40,1800x1169"),
            "qol-alt-tab-picker@Mon 0"
        );
    }

    #[test]
    fn launcher_events_get_pretty_text_and_compact_titles() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        runner.push_monitor((0, -40, 1800, 1169));

        let show = runner.format_launcher_event(
            "LAUNCHER_SHOW",
            "path=reuse title=qol-launcher@0,-40,1800x1169 pos=(0,-40) size=500x43 target=Focus",
        );
        assert!(show.contains("Launcher show"));
        assert!(show.contains("path=reuse") || show.contains("reuse"));
        assert!(show.contains("qol-launcher@Mon 0"));
        assert!(show.contains("500x43@(0,-40)"));

        let input = runner.format_launcher_event(
            "LAUNCHER_INPUT",
            "key=d effect=query title=qol-launcher q=\"doc\" selected=1 results_before=8",
        );
        assert!(input.contains("Launcher input"));
        assert!(input.contains("-> query"));
        assert!(input.contains("q=\"doc\""));
    }

    #[test]
    fn winact_done_marks_partial_after_failed_ax() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        let failed_ax = RawLine {
            ts_ms: 1,
            pid: "42".to_string(),
            tag: "WINACT_AX".to_string(),
            msg: "op=ax_raise pid=4242 dur_ms=7 outcome=fail".to_string(),
        };
        let done = RawLine {
            ts_ms: 2,
            pid: "42".to_string(),
            tag: "WINACT_DONE".to_string(),
            msg: "action=close total_ms=13 outcome=ok".to_string(),
        };

        assert!(runner
            .format_event(failed_ax)
            .unwrap()
            .text
            .contains("fail"));
        assert!(runner
            .format_event(done)
            .unwrap()
            .text
            .contains("partial: an AX op failed"));
    }

    #[test]
    fn monitor_events_get_python_style_labels() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        let publish = runner
            .format_event(RawLine {
                ts_ms: 1,
                pid: "1".to_string(),
                tag: "PUBLISH".to_string(),
                msg: "AMC idx=0 \"C\" is_boot=true -> delivered=[launcher] missed=[lights:unsubscribed]"
                    .to_string(),
            })
            .unwrap();
        assert_eq!(
            strip_ansi(&publish.text),
            "PUBLISH AMC idx=0 \"C\" is_boot=true -> delivered=[launcher] missed=[lights:unsubscribed]"
        );

        let recv = runner
            .format_event(RawLine {
                ts_ms: 2,
                pid: "1".to_string(),
                tag: "RECV".to_string(),
                msg: "AMC idx=0 \"C\" src=host".to_string(),
            })
            .unwrap();
        assert_eq!(strip_ansi(&recv.text), "RECV AMC idx=0 \"C\" src=host");
    }

    #[test]
    fn plugin_filter_keeps_publish_events_that_mention_plugin() {
        let args = Args::parse(&["launcher".into()]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        runner.process_raw(RawLine {
            ts_ms: 1,
            pid: "1".to_string(),
            tag: "PUBLISH".to_string(),
            msg:
                "AMC idx=0 \"C\" is_boot=true -> delivered=[launcher] missed=[lights:unsubscribed]"
                    .to_string(),
        });
        assert_eq!(runner.buffer.len(), 1);

        runner.process_raw(RawLine {
            ts_ms: 2,
            pid: "1".to_string(),
            tag: "PUBLISH".to_string(),
            msg: "AMC idx=0 \"C\" is_boot=true -> delivered=[alt-tab] missed=[]".to_string(),
        });
        assert_eq!(runner.buffer.len(), 1);
    }

    #[test]
    fn focus_events_get_pretty_labels() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        let activate = runner
            .format_event(RawLine {
                ts_ms: 1,
                pid: "1".to_string(),
                tag: "ACTIVATE".to_string(),
                msg: "#7 wid=99 title=\"Firefox\" source=2(qol) sent_ts=123 requestor_active=0"
                    .to_string(),
            })
            .unwrap();
        assert!(strip_ansi(&activate.text).contains("ACTIVATE #7: focus \"Firefox\""));

        let focus = runner
            .format_event(RawLine {
                ts_ms: 2,
                pid: "1".to_string(),
                tag: "FOCUS".to_string(),
                msg: "#7 result=OK active_after=99 demands_attention=false elapsed=14ms"
                    .to_string(),
            })
            .unwrap();
        assert!(strip_ansi(&focus.text).contains("FOCUS #7 SUCCESS"));
    }

    #[test]
    fn focus_fixture_resolves_pending_activation_success() {
        let args = Args::parse(&["focus".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 ACTIVATE_WIN wid=55 title=\"Target App\" source=2 timestamp=77 requester_active=0",
                "1120 pid=10 FOCUS_WIN winpos=(0,0,100x100) wid=55 title=\"Target App\" detect_lag_ms=5 ignored=false",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("ACTIVATE REQUEST: focus \"Target App\""));
        assert!(output.contains("FOCUS SUCCESS: Focused \"Target App\""));
        assert!(output.contains("120ms"));
        assert!(output.contains("detect ±5ms"));
        assert!(output.contains("➔ ACTIVATE REQUEST"));
        assert!(output.contains("✔ FOCUS SUCCESS"));
    }

    #[test]
    fn focus_fixture_marks_misdirected_focus() {
        let args = Args::parse(&["focus".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 ACTIVATE_WIN wid=55 title=\"Target App\" source=2 timestamp=77 requester_active=0",
                "1090 pid=10 FOCUS_WIN winpos=(0,0,100x100) wid=99 title=\"Other App\" ignored=false",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("MISDIRECTED FOCUS"));
        assert!(output.contains("⚠ MISDIRECTED FOCUS"));
        assert!(output.contains("Requested \"Target App\" (wid: 55)"));
        assert!(output.contains("focused \"Other App\" (wid: 99)"));
    }

    #[test]
    fn focus_fixture_times_out_pending_activation() {
        let args = Args::parse(&["focus".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 ACTIVATE_WIN wid=55 title=\"Target App\" source=2 timestamp=77 requester_active=0",
                "1701 pid=10 PICK cursor=(0,0) cursor_age_ms=1 focus=(0,0) focus_age_ms=2 winner=cursor",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("FOCUS FAILURE"));
        assert!(output.contains("✖ FOCUS FAILURE"));
        assert!(output.contains("Timed out focusing \"Target App\" (wid: 55)"));
    }

    #[test]
    fn focus_fixture_reports_confirmed_front_without_focus_win() {
        let args = Args::parse(&["focus".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 ACTIVATE_WIN wid=55 title=\"Target App\" source=2 timestamp=77 requester_active=0",
                "1100 pid=10 ACTIVATE_SETTLED wid=55",
                "1701 pid=10 PICK cursor=(0,0) cursor_age_ms=1 focus=(0,0) focus_age_ms=2 winner=cursor",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("FOCUS OK"));
        assert!(output.contains("✔ FOCUS OK"));
        assert!(output.contains("\"Target App\" (wid: 55) confirmed front"));
    }

    #[test]
    fn focus_fixture_marks_superseded_activation() {
        let args = Args::parse(&["focus".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 ACTIVATE_WIN wid=55 title=\"First App\" source=2 timestamp=77 requester_active=0",
                "1040 pid=10 ACTIVATE_WIN wid=99 title=\"Second App\" source=2 timestamp=78 requester_active=0",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("SUPERSEDED"));
        assert!(output.contains("⚠ SUPERSEDED"));
        assert!(output.contains("New request to focus \"Second App\""));
        assert!(output.contains("before focus on \"First App\" was confirmed"));
    }

    #[test]
    fn stats_fixture_counts_focus_outcomes() {
        let args = Args::parse(&["focus".into(), "--stats".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let _ = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 ACTIVATE_WIN wid=55 title=\"Target App\" source=2 timestamp=77 requester_active=0",
                "1120 pid=10 FOCUS_WIN winpos=(0,0,100x100) wid=55 title=\"Target App\" ignored=false",
                "2000 pid=10 ACTIVATE_WIN wid=66 title=\"Second App\" source=2 timestamp=78 requester_active=0",
                "2090 pid=10 FOCUS_WIN winpos=(0,0,100x100) wid=77 title=\"Other App\" ignored=false",
                "3000 pid=10 ACTIVATE_WIN wid=88 title=\"Third App\" source=2 timestamp=79 requester_active=0",
                "3701 pid=10 PICK cursor=(0,0) cursor_age_ms=1 focus=(0,0) focus_age_ms=2 winner=cursor",
            ],
        );
        let stats = strip_ansi(&runner.stats_text().expect("stats text"));
        assert!(stats.contains("Focus requests sent:  3"));
        assert!(stats.contains("Focus resolved:       3"));
        assert!(stats.contains("✔ success      1"));
        assert!(stats.contains("⚠ misdirected  1"));
        assert!(stats.contains("✖ timed out    1"));
        assert!(stats.contains("Focus latency ms: p50=120 p95=120 max=120 min=120 (n=1)"));
    }

    #[test]
    fn picker_status_events_are_deduped_until_state_changes() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        let ready = || RawLine {
            ts_ms: 1,
            pid: "1".to_string(),
            tag: "PICKER_READY".to_string(),
            msg: "title=qol-alt-tab-picker".to_string(),
        };
        let stale = || RawLine {
            ts_ms: 2,
            pid: "1".to_string(),
            tag: "PICKER_STALE".to_string(),
            msg: "title=qol-alt-tab-picker".to_string(),
        };

        assert!(runner.format_event(ready()).is_some());
        assert!(runner.format_event(ready()).is_none());
        assert!(runner.format_event(stale()).is_some());
    }

    #[test]
    fn opacity_fixture_skips_redundant_writes_and_marks_reverts() {
        let args = Args::parse(&["--topic=opacity".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 HIDE_WIN title=qol-alt-tab-picker@0,0,1800x1169 wid=11 path=compositor opacity=0 applied=true reason=open",
                "1010 pid=10 HIDE_WIN title=qol-alt-tab-picker@0,0,1800x1169 wid=11 path=compositor opacity=0 applied=true reason=open",
                "1050 pid=10 SHOW_WIN title=qol-alt-tab-picker@0,0,1800x1169 wid=11 cleared_opacity->1 source=1 timestamp=0 requester_active=0 reason=close",
                "1100 pid=10 HIDE_WIN title=qol-alt-tab-picker@0,0,1800x1169 wid=11 path=compositor opacity=0 applied=true reason=open",
            ],
        );
        let output = strip_ansi(&output);
        assert_eq!(output.matches("qol-alt-tab-picker").count(), 3);
        assert!(output.contains("qol-alt-tab-picker@Mon 0 -> 0.0"));
        assert!(output.contains("qol-alt-tab-picker@Mon 0 -> 1.0"));
        assert!(output.contains("REVERT close@50ms"));
        assert!(output.contains("⟲ REVERT close@50ms"));

        let waste = strip_ansi(&runner.opacity_waste_text().expect("opacity waste text"));
        assert!(waste.contains("OPACITY CHURN"));
        assert!(waste.contains("Opacity writes:      4"));
        assert!(waste.contains("Redundant (no-op):   1 (25%)"));
        assert!(waste.contains("Reverts (self-heal): 1"));
        assert!(waste.contains("open       3  (1 redundant)"));
        assert!(waste.contains("close      1"));
        assert!(waste.contains("close->open"));
        assert!(waste.contains("×1"));
    }

    #[test]
    fn opacity_fixture_drops_non_python_visibility_shapes() {
        let args = Args::parse(&["--topic=opacity".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "cli-sessions");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 HIDE_WIN title=cli-sessions-panel path=hidden reason=row-click",
                "1250 pid=10 SHOW_WIN title=cli-sessions-panel reason=shortcut",
                "1500 pid=10 HIDE_WIN title=cli-sessions-panel path=rest alpha=0.25 reason=debug",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.is_empty());
        assert!(runner.opacity_waste_text().is_none());
    }

    #[test]
    fn show_win_without_cleared_opacity_does_not_mutate_waste() {
        let args = Args::parse(&["--topic=opacity".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 HIDE_WIN title=qol-alt-tab-picker@0,0,1800x1169 wid=11 path=compositor opacity=0 applied=true reason=open",
                "1010 pid=10 SHOW_WIN title=qol-alt-tab-picker@0,0,1800x1169 wid=11 reason=shortcut",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("qol-alt-tab-picker@Mon 0 -> 0.0"));
        assert!(!output.contains("-> 1.0"));

        let waste = strip_ansi(&runner.opacity_waste_text().expect("opacity waste text"));
        assert!(waste.contains("Opacity writes:      1"));
        assert!(!waste.contains("shortcut"));
    }

    #[test]
    fn golden_focus_fixture_keeps_python_pretty_shape() {
        let args = Args::parse(&["focus".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = strip_ansi(&replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 ACTIVATE_WIN wid=55 title=\"Target App\" source=2 timestamp=77 requester_active=0",
                "1120 pid=10 FOCUS_WIN winpos=(0,0,100x100) wid=55 title=\"Target App\" detect_lag_ms=5 ignored=false",
            ],
        ));
        let first_ts = format_timestamp(1000);
        let second_ts = format_timestamp(1120);
        assert_eq!(
            output,
            format!(
                "[{first_ts}] ┌── [alt-tab] ➔ ACTIVATE REQUEST: focus \"Target App\" (wid: 55) (EWMH: source=2, timestamp=77, active=0) (span: 120ms)\n\
                 [{second_ts}] │   └── [host] ✔ FOCUS SUCCESS: Focused \"Target App\" (wid: 55) in 120ms (detect ±5ms)\n"
            )
        );
    }

    #[test]
    fn ghost_dump_flags_macos_clickable_ghost_divergence() {
        let args = Args::parse(&["--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 GHOST_DUMP ctx=(idle title=qol-alt-tab-picker) title=\"qol-alt-tab-picker\" alpha=1.00 level=101 mouse_ignored=false frame=1800x1169@0,-40",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("DIVERGENCE (opaque clickable ghost outside show)"));
        assert!(output.contains("\"qol-alt-tab-picker\" alpha=1.0"));
    }

    #[test]
    fn ghost_dump_normalizes_alpha_like_python_float() {
        let args = Args::parse(&["--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 GHOST_DUMP ctx=(show title=qol-alt-tab-picker) title=\"qol-alt-tab-picker\" alpha=1.00 level=101 mouse_ignored=false frame=1800x1169@0,-40",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("\"qol-alt-tab-picker\" alpha=1.0"));
        assert!(!output.contains("alpha=1.00"));
    }

    #[test]
    fn ghost_dump_drops_nested_ctx_like_python_reference() {
        let args = Args::parse(&["--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "launcher");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 GHOST_DUMP ctx=(reconcile target=qol-launcher@0,0,1920x1080 active_mon=Some(ActiveMonitor { inner: MonitorBounds { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 } })) title=\"qol-launcher@0,0,1920x1080\" alpha=0.00 level=101 mouse_ignored=true frame=500x42@710,692",
            ],
        );
        assert!(strip_ansi(&output).is_empty());
    }

    #[test]
    fn no_ghosts_keeps_flat_ghost_dump_like_python_reference() {
        let args = Args::parse(&["--details".into(), "--no-ghosts".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 GHOST_DUMP ctx=(idle title=qol-alt-tab-picker) title=\"qol-alt-tab-picker\" alpha=1.00 level=101 mouse_ignored=false frame=1800x1169@0,-40",
            ],
        );
        assert!(strip_ansi(&output).contains("DIVERGENCE (opaque clickable ghost outside show)"));
    }

    #[test]
    fn cycle_drops_quoted_title_that_python_reference_cannot_parse() {
        let args = Args::parse(&["--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "alt-tab");
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 CYCLE method=tab from=2 to=3 count=6 to_app=\"Firefox Developer Edition\" to_title=\"So I \"Finished\" DELTARUNE...\" elapsed_ms=6",
            ],
        );
        assert!(strip_ansi(&output).is_empty());
    }

    #[test]
    fn linux_ghost_dump_summarizes_active_picker() {
        let args = Args::parse(&["--topic=opacity".into(), "--details".into()]).unwrap();
        let mut runner = runner_with_pid(args, "10", "host");
        runner
            .pid_names
            .insert("77".to_string(), "alt-tab".to_string());
        runner.push_monitor((0, 0, 100, 100));
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 SHOW_WIN title=qol-alt-tab-picker@0,0,100x100 wid=5 cleared_opacity->1 reason=open",
                "1010 pid=10 GHOSTDUMP begin",
                "1011 pid=10 GHOSTWIN title=qol-alt-tab-picker@0,0,100x100 owner_pid=77 wid=5 pos=(0,0) size=100x100 opacity=1 map=viewable role=live",
                "1020 pid=10 GHOSTDUMP end",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("Active Picker: qol-alt-tab-picker@Mon 0/alt-tab(1.0)[stale]"));
        assert!(output.contains("OK"));
    }

    #[test]
    fn linux_ghost_dump_reports_divergence_and_survives_plugin_filter() {
        let args = Args::parse(&[
            "alt-tab".into(),
            "--topic=opacity".into(),
            "--details".into(),
        ])
        .unwrap();
        let mut runner = runner_with_pid(args, "10", "host");
        runner
            .pid_names
            .insert("77".to_string(), "alt-tab".to_string());
        runner.push_monitor((0, 0, 100, 100));
        runner.push_monitor((100, 0, 100, 100));
        let output = replay_fixture(
            &mut runner,
            &[
                "1000 pid=10 GHOSTDUMP begin",
                "1010 pid=10 GHOSTWIN title=qol-alt-tab-picker@100,0,100x100 owner_pid=77 wid=5 pos=(0,0) size=100x100 opacity=1 map=viewable role=ghost",
                "1020 pid=10 GHOSTDUMP end",
            ],
        );
        let output = strip_ansi(&output);
        assert!(output.contains("Active Ghost: qol-alt-tab-picker@Mon 1/alt-tab(1.0)[stale]"));
        assert!(output.contains("DIVERGENCE"));
        assert!(output.contains("expected 0.0"));
        assert!(output.contains("is on Mon 0 (expected Mon 1)"));
    }

    fn runner_with_pid(args: Args, pid: &str, process_name: &str) -> TraceRunner {
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        runner
            .pid_names
            .insert(pid.to_string(), process_name.to_string());
        runner
    }

    fn replay_fixture(runner: &mut TraceRunner, lines: &[&str]) -> String {
        let mut prev_ts = None;
        let mut output = String::new();
        for line in lines {
            let raw = parse_raw_line(line).expect("fixture line");
            if prev_ts.is_some_and(|prev| raw.ts_ms.saturating_sub(prev) > REPLAY_GAP_MS) {
                if let Some(group) = runner.flush_text() {
                    output.push_str(&group);
                    output.push('\n');
                }
            }
            prev_ts = Some(raw.ts_ms);
            runner.process_raw(raw);
        }
        if let Some(group) = runner.flush_text() {
            output.push_str(&group);
        }
        output
    }

    fn strip_ansi(value: &str) -> String {
        let mut out = String::new();
        let mut chars = value.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for code_ch in chars.by_ref() {
                    if code_ch.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
            out.push(ch);
        }
        out
    }

    #[test]
    fn toggle_details_switches_trace_group_mode() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        assert!(!runner.args.details);
        assert!(strip_ansi(&runner.toggle_details()).contains("Trace details: expanded"));
        assert!(runner.args.details);
        assert!(strip_ansi(&runner.toggle_details()).contains("Trace details: collapsed"));
        assert!(!runner.args.details);
    }
}
