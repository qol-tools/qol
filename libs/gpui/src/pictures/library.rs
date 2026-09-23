use qol_theme::accent_swatch;

use super::svg::{self, WebPalette, WindowPalette, FILL, SOFT};
use super::PictureContext;

pub(crate) fn split_spec(spec: &str) -> (&str, &str) {
    match spec.split_once(':') {
        Some((name, argument)) => (name, argument),
        None => (spec, ""),
    }
}

pub(crate) fn markup_for(spec: &str, context: &PictureContext) -> Option<String> {
    let (name, argument) = split_spec(spec);
    let picture = match name {
        "hold-to-switch" => hold_to_switch(),
        "sticky" => sticky(),
        "cycle-once" => cycle_once(),
        "show-only" => show_only(),
        "icon-corner" => icon_corner(argument == "right"),
        "aa" => aa(argument.parse().ok()?),
        "desktop-theme" => desktop_theme(context, argument),
        "web-theme" => web_theme(context, argument),
        "swatch" => swatch(accent_swatch(context.mode, argument)?),
        "qol-toasts" => qol_toasts(),
        "system-bubbles" => system_bubbles(),
        "both-notices" => both_notices(),
        "launch-app" => launch_app(),
        "open-url" => open_url(),
        "bundle-ref" => bundle_ref("com.example.App"),
        "path-ref" => path_ref("\u{2026}/App.app"),
        "name-ref" => name_ref("App Name"),
        "browser-bundle-ref" => browser_bundle_ref(),
        "browser-path-ref" => browser_path_ref(),
        "browser-name-ref" => browser_name_ref(),
        "letters" => plugin(argument),
        "next-window" => next_window(),
        "previous-window" => previous_window(),
        "gear" => gear(),
        "adapter-auto" => adapter_auto(),
        "adapter-chip" => adapter_chip(),
        "schedule-off" => schedule_off(),
        "schedule-daily" => schedule_daily(),
        "mic-default" => mic_default(),
        "usb-mic" => usb_mic(),
        "webcam" => webcam(),
        "headset" => headset(),
        "speaker-default" => speaker_default(),
        "hdmi" => hdmi(),
        "headphones" => headphones(),
        "preset" => preset(argument)?,
        "format" => format_picture(argument),
        "copy-image" => copy_image(),
        "copy-path" => copy_path(),
        "no-terminal" => no_terminal(),
        "terminal-session" => terminal_session(argument),
        "insert-only" => insert_only(),
        "insert-submit" => insert_submit(),
        "prefer-local" => prefer_local(),
        "local-engine" => local_engine(argument),
        "remote-engine" => remote_engine(),
        "no-model" => no_model(),
        "model" => model(),
        "family-auto" => family_auto(),
        "fixed-size" => fixed_size(),
        "relative-size" => relative_size(),
        "display-mode" => display_mode(argument)?,
        _ => return None,
    };
    Some(picture)
}

fn hold_to_switch() -> String {
    let mut out = svg::rect(3.0, 22.0, 19.0, 16.0, 2.5, SOFT);
    out.push_str(&svg::rect(74.0, 22.0, 19.0, 16.0, 2.5, SOFT));
    out.push_str(&svg::wash(26.0, 12.0, 44.0, 36.0, 3.5, 0.22));
    out.push_str(&svg::rect(
        26.0,
        12.0,
        44.0,
        36.0,
        3.5,
        " stroke-width=\"2.2\"",
    ));
    out.push_str(&svg::line(26.0, 19.0, 70.0, 19.0, SOFT));
    svg::svg(&out)
}

fn sticky() -> String {
    let mut out = svg::rect(3.0, 18.0, 86.0, 32.0, 4.5, "");
    for (index, x) in [9.0, 36.0, 63.0].into_iter().enumerate() {
        if index == 1 {
            out.push_str(&svg::wash(x, 24.0, 20.0, 20.0, 2.5, 0.22));
        }
        let more = if index == 1 {
            " stroke-width=\"2.2\""
        } else {
            SOFT
        };
        out.push_str(&svg::rect(x, 24.0, 20.0, 20.0, 2.5, more));
    }
    out.push_str(&svg::pin(86.0, 24.0, 20.0));
    svg::svg(&out)
}

fn cycle_once() -> String {
    svg::svg(&(svg::hop_right() + &svg::strip(1, 28.0)))
}

fn show_only() -> String {
    let eye = svg::path("M11 16 Q20 8 29 16 Q20 24 11 16 Z", "");
    svg::svg(&(eye + &svg::dot(20.0, 16.0, 2.4) + &svg::strip(0, 28.0)))
}

fn icon_corner(right: bool) -> String {
    let mut out = svg::wash(18.0, 8.0, 60.0, 44.0, 4.0, 0.14);
    out.push_str(&svg::rect(18.0, 8.0, 60.0, 44.0, 4.0, ""));
    out.push_str(&svg::line(24.0, 44.0, 56.0, 44.0, SOFT));
    let corner = if right { 63.0 } else { 24.0 };
    out.push_str(&svg::rect(corner, 13.0, 9.0, 9.0, 2.0, FILL));
    svg::svg(&out)
}

fn aa(px: f64) -> String {
    let caption = svg::text(
        48.0,
        30.0 + px * 0.36,
        "Aa",
        px,
        "middle",
        "IBM Plex Sans, sans-serif",
        600,
    );
    svg::svg(&caption)
}

fn desktop_theme(context: &PictureContext, argument: &str) -> String {
    let preview = if argument == "bone" {
        &context.bone
    } else {
        &context.slate
    };
    let palette = WindowPalette {
        pane: svg::hex(preview.pane),
        rail: svg::hex(preview.rail),
        edge: svg::hex(preview.edge),
        ink: svg::hex(preview.ink),
        soft: svg::hex(preview.soft),
        band: svg::hex(preview.band),
        accent: svg::hex(preview.accent),
    };
    svg::svg(&svg::app_window(&palette))
}

fn web_theme(context: &PictureContext, argument: &str) -> String {
    let preview = if argument == "midnight" {
        &context.web_midnight
    } else {
        &context.web_slate
    };
    let palette = WebPalette {
        bg: svg::hex(preview.bg),
        surface: svg::hex(preview.surface),
        raised: svg::hex(preview.raised),
        border: svg::hex(preview.border),
        text: svg::hex(preview.text),
        muted: svg::hex(preview.muted),
    };
    svg::svg(&svg::web_window(&palette))
}

fn swatch(value: u32) -> String {
    let fill = format!(" fill=\"{}\" stroke=\"none\"", svg::hex(value));
    svg::svg(&svg::circle(48.0, 30.0, 16.0, &fill))
}

fn qol_toasts() -> String {
    svg::svg(&(svg::screen() + &svg::toast(50.0, 34.0)))
}

fn system_bubbles() -> String {
    svg::svg(&(svg::screen() + &svg::bubble(50.0, 10.0)))
}

fn both_notices() -> String {
    svg::svg(&(svg::screen() + &svg::bubble(50.0, 10.0) + &svg::toast(50.0, 34.0)))
}

fn launch_app() -> String {
    let mut out = svg::wash(26.0, 14.0, 14.0, 14.0, 3.0, 0.35);
    out.push_str(&svg::rect(26.0, 14.0, 14.0, 14.0, 3.0, ""));
    out.push_str(&svg::rect(44.0, 14.0, 14.0, 14.0, 3.0, ""));
    out.push_str(&svg::rect(26.0, 32.0, 14.0, 14.0, 3.0, ""));
    out.push_str(&svg::rect(44.0, 32.0, 14.0, 14.0, 3.0, ""));
    out.push_str(&svg::path("M66 36 L80 22 M72 22 H80 V30", ""));
    svg::svg(&out)
}

fn open_url() -> String {
    let mut out = svg::rect(10.0, 8.0, 76.0, 44.0, 4.0, "");
    out.push_str(&svg::line(10.0, 18.0, 86.0, 18.0, ""));
    out.push_str(&svg::rect(22.0, 10.5, 52.0, 5.0, 2.5, SOFT));
    out.push_str(&mono(48.0, 38.0, "https://", 10.0, "middle"));
    svg::svg(&out)
}

fn bundle_ref(text: &str) -> String {
    let mut out = svg::path("M10 20 H74 L84 30 L74 40 H10 Z", "");
    out.push_str(&mono(43.0, 32.5, text, 7.0, "middle"));
    svg::svg(&out)
}

fn path_ref(text: &str) -> String {
    svg::svg(&(svg::folder("") + &mono(48.0, 38.0, text, 7.0, "middle")))
}

fn name_ref(text: &str) -> String {
    let mut out = svg::rect(12.0, 20.0, 72.0, 20.0, 4.0, "");
    out.push_str(&mono(45.0, 33.5, text, 9.0, "middle"));
    out.push_str(&svg::line(76.0, 24.0, 76.0, 36.0, ""));
    svg::svg(&out)
}

fn browser_bundle_ref() -> String {
    let mut out = svg::rect(16.0, 12.0, 64.0, 38.0, 4.0, "");
    out.push_str(&svg::rect(42.0, 16.0, 12.0, 3.0, 1.5, ""));
    out.push_str(&svg::wash(22.0, 23.0, 20.0, 20.0, 3.0, 0.2));
    out.push_str(&svg::globe(32.0, 33.0, 7.0));
    out.push_str(&svg::line(48.0, 27.0, 72.0, 27.0, ""));
    out.push_str(&svg::line(48.0, 34.0, 68.0, 34.0, SOFT));
    out.push_str(&svg::line(48.0, 41.0, 62.0, 41.0, SOFT));
    svg::svg(&out)
}

fn browser_path_ref() -> String {
    let mut out = svg::rect(8.0, 19.0, 80.0, 22.0, 5.0, "");
    out.push_str(&svg::mini_folder(18.0, 25.5));
    out.push_str(&svg::chevron(34.0, 30.0));
    out.push_str(&svg::mini_folder(41.0, 25.5));
    out.push_str(&svg::chevron(57.0, 30.0));
    out.push_str(&svg::globe(72.0, 30.0, 6.0));
    svg::svg(&out)
}

fn browser_name_ref() -> String {
    let mut out = svg::rect(10.0, 19.0, 76.0, 22.0, 5.0, "");
    out.push_str(&svg::globe(23.0, 30.0, 6.0));
    let name = svg::text(
        34.0,
        33.5,
        "Firefox",
        10.0,
        "start",
        "IBM Plex Sans, sans-serif",
        500,
    );
    out.push_str(&name);
    out.push_str(&svg::line(68.0, 24.5, 68.0, 35.5, ""));
    svg::svg(&out)
}

fn plugin(text: &str) -> String {
    svg::svg(&svg::letters(&escape(text)))
}

fn next_window() -> String {
    svg::svg(&(svg::hop_right() + &svg::strip(1, 28.0)))
}

fn previous_window() -> String {
    svg::svg(&(svg::hop_left() + &svg::strip(1, 28.0)))
}

fn gear() -> String {
    let mut points: Vec<String> = Vec::new();
    for angle in [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        for (offset, radius) in [(-12.0, 12.5), (-7.0, 17.0), (7.0, 17.0), (12.0, 12.5)] {
            points.push(gear_point(angle + offset, radius));
        }
    }
    let outline = format!("M{} Z", points.join(" L"));
    svg::svg(&(svg::path(&outline, "") + &svg::circle(48.0, 30.0, 5.5, "")))
}

fn gear_point(degrees: f64, radius: f64) -> String {
    let radians = (degrees * std::f64::consts::PI) / 180.0;
    let x = svg::n(48.0 + radians.cos() * radius);
    let y = svg::n(30.0 + radians.sin() * radius);
    format!("{x} {y}")
}

fn adapter_auto() -> String {
    svg::svg(&(svg::screen() + &svg::rune(48.0, 28.0, 1.3)))
}

fn adapter_chip() -> String {
    svg::svg(&(svg::chip_frame() + &svg::rune(48.0, 30.0, 1.05)))
}

fn schedule_off() -> String {
    svg::svg(&svg::dial(false, " fill-opacity=\".45\""))
}

fn schedule_daily() -> String {
    svg::svg(&svg::dial(true, ""))
}

fn mic_default() -> String {
    let scaled = svg::shrink(&svg::mic(0.0), 0.62, 48.0, 28.0, 48.0, 28.0);
    svg::svg(&(svg::screen() + &scaled))
}

fn usb_mic() -> String {
    svg::svg(&(svg::desk_mic(-9.0) + &svg::usb_logo(66.0)))
}

fn webcam() -> String {
    let mut out = svg::rect(26.0, 12.0, 44.0, 28.0, 6.0, "");
    out.push_str(&svg::circle(48.0, 26.0, 8.0, ""));
    out.push_str(&svg::dot(48.0, 26.0, 2.6));
    out.push_str(&svg::line(48.0, 40.0, 48.0, 48.0, ""));
    out.push_str(&svg::line(38.0, 48.0, 58.0, 48.0, ""));
    svg::svg(&out)
}

fn headset() -> String {
    let mut cutout = svg::rect(
        66.0,
        30.0,
        13.0,
        22.0,
        5.0,
        " fill=\"#000\" stroke=\"none\"",
    );
    cutout.push_str(&svg::rect(
        45.0,
        52.5,
        9.0,
        5.5,
        2.75,
        " fill=\"#000\" stroke=\"none\"",
    ));
    let booth = svg::rect(
        45.0,
        52.5,
        9.0,
        5.5,
        2.75,
        " fill=\"currentColor\" fill-opacity=\".35\"",
    );
    let mut out = svg::cans();
    out.push_str(&svg::masked(
        &svg::path("M68 43 Q62 54 51 55.2", ""),
        &cutout,
    ));
    out.push_str(&booth);
    svg::svg(&out)
}

fn speaker_default() -> String {
    let scaled = svg::shrink(&svg::speaker(), 0.62, 44.0, 30.0, 48.0, 28.0);
    svg::svg(&(svg::screen() + &scaled))
}

fn hdmi() -> String {
    let mut out = svg::rect(12.0, 10.0, 52.0, 32.0, 3.0, "");
    out.push_str(&svg::line(32.0, 48.0, 44.0, 48.0, ""));
    out.push_str(&svg::line(38.0, 42.0, 38.0, 48.0, ""));
    out.push_str(&svg::path("M72 22 Q77 28 72 34", ""));
    out.push_str(&svg::path("M78 17 Q86 28 78 39", SOFT));
    svg::svg(&out)
}

fn headphones() -> String {
    svg::svg(&svg::cans())
}

fn preset(argument: &str) -> Option<String> {
    let (speed, size) = argument.split_once(',')?;
    let speed = speed.parse::<f64>().ok()? / 100.0;
    let size = size.parse::<f64>().ok()? / 100.0;
    Some(preset_levels(speed, size))
}

fn preset_levels(speed: f64, size: f64) -> String {
    let mut out = svg::path("M18 7 L11 18 H16 L14 26 L22 14 H17 Z", "");
    out.push_str(&svg::bar(12.0, speed));
    out.push_str(&svg::path("M12 34 H18 L22 38 V48 H12 Z", ""));
    out.push_str(&svg::bar(37.0, size));
    svg::svg(&out)
}

fn format_picture(ext: &str) -> String {
    let mut out = svg::path("M28 5 H58 L68 15 V55 H28 Z", "");
    out.push_str(&svg::path("M58 5 V15 H68", ""));
    out.push_str(&svg::path("M43 20 L43 32 L53 26 Z", FILL));
    out.push_str(&mono(48.0, 46.0, &escape(ext), 9.0, "middle"));
    svg::svg(&out)
}

fn copy_image() -> String {
    let mut front = svg::path("M24 44 L34 32 L42 40 L48 34 L60 44", "");
    front.push_str(&svg::circle(53.0, 24.0, 3.0, ""));
    svg::svg(&svg::copies(&front))
}

fn copy_path() -> String {
    let mut front = mono(42.0, 29.0, "~/Pictures", 7.0, "middle");
    front.push_str(&mono(42.0, 40.0, "shot.png", 7.0, "middle"));
    svg::svg(&svg::copies(&front))
}

fn no_terminal() -> String {
    let prompt = mono(18.0, 38.0, "&gt;_", 11.0, "start");
    svg::svg(&svg::slashed(
        &svg::terminal(&prompt),
        18.0,
        5.0,
        78.0,
        55.0,
    ))
}

fn terminal_session(name: &str) -> String {
    let prompt = format!("&gt; {}", escape(name));
    let mut out = svg::terminal(&mono(16.0, 30.0, &prompt, 8.0, "start"));
    out.push_str(&svg::line(16.0, 38.0, 62.0, 38.0, SOFT));
    out.push_str(&svg::line(16.0, 45.0, 48.0, 45.0, SOFT));
    svg::svg(&out)
}

fn insert_only() -> String {
    let mut out = svg::terminal(&mono(16.0, 34.0, "&gt; hello", 9.0, "start"));
    out.push_str(&svg::rect(57.0, 26.0, 2.0, 11.0, 0.0, FILL));
    svg::svg(&out)
}

fn insert_submit() -> String {
    let mut out = svg::terminal(&mono(16.0, 34.0, "&gt; hello", 9.0, "start"));
    out.push_str(&svg::path("M73 24 V28.5 Q73 32 69.5 32 H60", ""));
    out.push_str(&svg::path("M63.5 28.5 L60 32 L63.5 35.5", ""));
    svg::svg(&out)
}

fn prefer_local() -> String {
    let mut out = svg::dot(12.0, 30.0, 2.4);
    out.push_str(&svg::path("M14 30 H24 L33 18 H50", ""));
    out.push_str(&svg::path("M46.5 14.5 L50 18 L46.5 21.5", ""));
    out.push_str(&svg::path(
        "M24 30 L33 42 H50",
        " stroke-dasharray=\"2.5 3.5\" stroke-opacity=\".5\"",
    ));
    out.push_str(&svg::rect(55.0, 9.0, 24.0, 18.0, 3.0, ""));
    for x in [61.0, 67.0, 73.0] {
        out.push_str(&svg::line(x, 5.5, x, 9.0, ""));
        out.push_str(&svg::line(x, 27.0, x, 30.5, ""));
    }
    out.push_str(&svg::path(
        "M60 50 H75 A5 5 0 0 0 75.5 40 A7 7 0 0 0 62.5 40.5 A4.75 4.75 0 0 0 60 50 Z",
        SOFT,
    ));
    svg::svg(&out)
}

fn local_engine(label: &str) -> String {
    svg::svg(&svg::chip(&escape(label)))
}

fn remote_engine() -> String {
    let mut out = svg::rect(12.0, 16.0, 24.0, 28.0, 3.0, "");
    out.push_str(&svg::line(17.0, 24.0, 31.0, 24.0, SOFT));
    out.push_str(&svg::line(17.0, 30.0, 31.0, 30.0, SOFT));
    out.push_str(&svg::rect(60.0, 16.0, 24.0, 28.0, 3.0, ""));
    out.push_str(&svg::line(65.0, 24.0, 79.0, 24.0, SOFT));
    out.push_str(&svg::line(65.0, 30.0, 79.0, 30.0, SOFT));
    out.push_str(&svg::path("M38 30 H58", " stroke-dasharray=\"3 3\""));
    out.push_str(&svg::path("M53 26 L58 30 L53 34", ""));
    svg::svg(&out)
}

fn no_model() -> String {
    svg::svg(&svg::folder(" stroke-dasharray=\"3 3\""))
}

fn model() -> String {
    let wave = "M24 34 L28 28 L32 40 L36 24 L40 36 L44 30 L48 34 L52 29 L56 38 L60 32 L64 34";
    svg::svg(&(svg::folder("") + &svg::path(wave, "")))
}

fn family_auto() -> String {
    let mut out = svg::folder(SOFT);
    out.push_str(&svg::circle(50.0, 32.0, 10.0, ""));
    out.push_str(&svg::line(57.5, 39.5, 65.0, 47.0, " stroke-width=\"2.6\""));
    let trace = "M42.5 32 L45 29 L47.5 35.5 L50 26.5 L52.5 36 L55 30 L57.5 32";
    out.push_str(&svg::path(trace, ""));
    svg::svg(&out)
}

fn fixed_size() -> String {
    let mut out = svg::rect(8.0, 6.0, 80.0, 46.0, 4.0, "");
    out.push_str(&svg::wash(28.0, 14.0, 40.0, 30.0, 2.0, 0.14));
    out.push_str(&svg::rect(28.0, 14.0, 40.0, 30.0, 2.0, ""));
    out.push_str(&mono(48.0, 32.0, "1152\u{d7}892", 7.0, "middle"));
    svg::svg(&out)
}

fn relative_size() -> String {
    let mut out = svg::rect(8.0, 6.0, 80.0, 46.0, 4.0, "");
    out.push_str(&svg::wash(22.5, 10.5, 51.0, 37.0, 2.0, 0.14));
    out.push_str(&svg::rect(22.5, 10.5, 51.0, 37.0, 2.0, ""));
    out.push_str(&mono(48.0, 32.5, "64%", 10.0, "middle"));
    svg::svg(&out)
}

fn display_mode(argument: &str) -> Option<String> {
    let (width, height) = argument.split_once('x')?;
    let width = width.parse::<u32>().ok()?;
    let height = height.parse::<u32>().ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let scale = (76.0 / width as f64).min(44.0 / height as f64);
    let screen_width = svg::rounded(width as f64 * scale);
    let screen_height = svg::rounded(height as f64 * scale);
    let x = svg::rounded(48.0 - screen_width / 2.0);
    let top = svg::rounded((60.0 - screen_height - 6.0) / 2.0);
    let bottom = svg::rounded(top + screen_height);
    let stand = svg::rounded(bottom + 6.0);
    let mut out = svg::rect(x, top, screen_width, screen_height, 4.0, "");
    out.push_str(&svg::line(48.0, bottom, 48.0, stand, ""));
    out.push_str(&svg::line(40.0, stand, 56.0, stand, ""));
    Some(svg::svg(&out))
}

fn mono(x: f64, y: f64, t: &str, size: f64, anchor: &str) -> String {
    svg::text(x, y, t, size, anchor, "IBM Plex Mono, monospace", 500)
}

pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}
