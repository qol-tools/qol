use std::cell::Cell;

pub(crate) const SOFT: &str = " stroke-opacity=\".5\"";
pub(crate) const FILL: &str = " fill=\"currentColor\" stroke=\"none\"";

pub(crate) const TICK_MARKUP: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"12\" viewBox=\"-6 0 16 12\" fill=\"none\"><path d=\"M7.5962 2.318L9.0104 3.7322L1.2322 11.5104L-3.0104 7.2678L-1.5962 5.8536L1.2322 8.682Z\" fill=\"currentColor\"/></svg>";

pub(crate) const CHEVRON_MARKUP: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"14\" viewBox=\"0 0 8 14\" fill=\"none\"><path d=\"M1.5 1.5L6.5 7L1.5 12.5\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/></svg>";

pub(crate) struct WindowPalette {
    pub(crate) pane: String,
    pub(crate) rail: String,
    pub(crate) edge: String,
    pub(crate) ink: String,
    pub(crate) soft: String,
    pub(crate) band: String,
    pub(crate) accent: String,
}

pub(crate) struct WebPalette {
    pub(crate) bg: String,
    pub(crate) surface: String,
    pub(crate) raised: String,
    pub(crate) border: String,
    pub(crate) text: String,
    pub(crate) muted: String,
}

thread_local! {
    static MASKS: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn reset_masks() {
    MASKS.with(|masks| masks.set(0));
}

fn next_mask_id() -> usize {
    MASKS.with(|masks| {
        let id = masks.get() + 1;
        masks.set(id);
        id
    })
}

pub(crate) fn hex(value: u32) -> String {
    format!("#{value:06x}")
}

pub(crate) fn number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    format!("{value}")
}

pub(crate) fn rounded(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let bits = value.to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i64;
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    let (mantissa, exponent) = if exponent_bits == 0 {
        (fraction as u128, -1074i64)
    } else {
        (
            (fraction | 0x0010_0000_0000_0000) as u128,
            exponent_bits - 1075,
        )
    };
    if exponent >= 0 {
        return value;
    }
    let shift = (-exponent) as u32;
    let hundredths = if shift >= 110 {
        0
    } else {
        let denominator = 1u128 << shift;
        (mantissa * 200 + denominator) / (2 * denominator)
    };
    let magnitude = hundredths as f64 / 100.0;
    if value.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    }
}

pub(crate) fn n(value: f64) -> String {
    number(rounded(value))
}

pub(crate) fn svg(inner: &str) -> String {
    let mut out = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\"");
    out.push_str(" width=\"96\" height=\"60\" viewBox=\"0 0 96 60\"");
    out.push_str(" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\"");
    out.push_str(" stroke-linecap=\"round\" stroke-linejoin=\"round\">");
    out.push_str(inner);
    out.push_str("</svg>");
    out
}

pub(crate) fn rect(x: f64, y: f64, w: f64, h: f64, r: f64, more: &str) -> String {
    format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\"{more}/>",
        number(x),
        number(y),
        number(w),
        number(h),
        number(r)
    )
}

pub(crate) fn wash(x: f64, y: f64, w: f64, h: f64, r: f64, o: f64) -> String {
    let mut more = String::from(" fill=\"currentColor\" fill-opacity=\"");
    more.push_str(&number(o));
    more.push_str("\" stroke=\"none\"");
    rect(x, y, w, h, r, &more)
}

pub(crate) fn line(x1: f64, y1: f64, x2: f64, y2: f64, more: &str) -> String {
    format!(
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"{more}/>",
        number(x1),
        number(y1),
        number(x2),
        number(y2)
    )
}

pub(crate) fn path(d: &str, more: &str) -> String {
    format!("<path d=\"{d}\"{more}/>")
}

pub(crate) fn circle(cx: f64, cy: f64, r: f64, more: &str) -> String {
    format!(
        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\"{more}/>",
        number(cx),
        number(cy),
        number(r)
    )
}

pub(crate) fn dot(cx: f64, cy: f64, r: f64) -> String {
    circle(cx, cy, r, FILL)
}

pub(crate) fn text(
    x: f64,
    y: f64,
    t: &str,
    size: f64,
    anchor: &str,
    family: &str,
    weight: u32,
) -> String {
    format!(
        "<text x=\"{}\" y=\"{}\" font-size=\"{}\" font-family=\"{family}\" font-weight=\"{weight}\" text-anchor=\"{anchor}\" fill=\"currentColor\" stroke=\"none\">{t}</text>",
        number(x),
        number(y),
        number(size)
    )
}

pub(crate) fn shrink(inner: &str, s: f64, fx: f64, fy: f64, tx: f64, ty: f64) -> String {
    format!(
        "<g transform=\"translate({} {}) scale({}) translate({} {})\" stroke-width=\"{}\">{inner}</g>",
        number(tx),
        number(ty),
        number(s),
        number(-fx),
        number(-fy),
        n(1.5 / s)
    )
}

pub(crate) fn strip(selected: usize, y: f64) -> String {
    let mut out = String::new();
    for (index, x) in [10.0, 38.0, 66.0].into_iter().enumerate() {
        out.push_str(&wash(x, y, 20.0, 14.0, 2.0, 0.14));
        let more = if index == selected {
            " stroke-width=\"2.4\""
        } else {
            SOFT
        };
        out.push_str(&rect(x, y, 20.0, 14.0, 2.0, more));
    }
    out
}

pub(crate) fn hop_right() -> String {
    path("M20 22 Q33 10 46 21", "") + &path("M41.5 20.5 L46 21 L45 16.5", "")
}

pub(crate) fn hop_left() -> String {
    path("M76 22 Q63 10 50 21", "") + &path("M54.5 20.5 L50 21 L51 16.5", "")
}

pub(crate) fn pin(x: f64, y: f64, angle: f64) -> String {
    let body = "M-3.45 0 H3.45 V1.38 H2.53 V5.29 Q2.53 7.59 4.83 8.28 V9.66 H-4.83 V8.28 Q-2.53 7.59 -2.53 5.29 V1.38 H-3.45 Z";
    let fill = " fill=\"currentColor\" fill-opacity=\".35\"";
    let pointer = path(body, fill);
    let stem = line(0.0, 9.66, 0.0, 15.0, "");
    format!(
        "<g transform=\"translate({} {}) rotate({}) translate(0 -15)\" stroke-width=\"1.3\">{pointer}{stem}</g>",
        number(x),
        number(y),
        number(angle)
    )
}

pub(crate) fn app_window(p: &WindowPalette) -> String {
    let first = format!(" fill=\"{}\" stroke=\"{}\"", p.pane, p.edge);
    let mut out = rect(10.0, 6.0, 76.0, 48.0, 4.0, &first);
    let rail = format!(" fill=\"{}\" stroke=\"none\"", p.rail);
    out.push_str(&rect(10.75, 6.75, 20.0, 46.5, 3.25, &rail));
    for (index, y) in [14.0, 20.0, 26.0].into_iter().enumerate() {
        let stroke = format!(" stroke=\"{}\"", p.soft);
        out.push_str(&line(15.0, y, 25.0 - index as f64, y, &stroke));
    }
    let band = format!(" fill=\"{}\" stroke=\"none\"", p.band);
    out.push_str(&rect(31.0, 21.0, 54.25, 10.0, 0.0, &band));
    let accent = format!(" stroke=\"{}\" stroke-width=\"2\"", p.accent);
    out.push_str(&line(32.0, 21.5, 32.0, 30.5, &accent));
    let ink = format!(" stroke=\"{}\"", p.ink);
    out.push_str(&line(37.0, 14.0, 66.0, 14.0, &ink));
    out.push_str(&line(37.0, 26.0, 62.0, 26.0, &ink));
    let soft = format!(" stroke=\"{}\"", p.soft);
    out.push_str(&line(37.0, 38.0, 70.0, 38.0, &soft));
    out.push_str(&line(37.0, 46.0, 58.0, 46.0, &soft));
    out
}

pub(crate) fn web_window(p: &WebPalette) -> String {
    let frame = format!(" fill=\"{}\" stroke=\"{}\"", p.bg, p.border);
    let mut out = rect(8.0, 6.0, 80.0, 48.0, 4.0, &frame);
    let surface = format!(" fill=\"{}\" stroke=\"none\"", p.surface);
    out.push_str(&rect(8.75, 6.75, 78.5, 9.0, 3.25, &surface));
    let muted = format!(" fill=\"{}\" stroke=\"none\"", p.muted);
    for x in [14.0, 19.0, 24.0] {
        out.push_str(&circle(x, 11.2, 1.2, &muted));
    }
    let raised = format!(" fill=\"{}\" stroke=\"none\"", p.raised);
    out.push_str(&rect(32.0, 9.0, 40.0, 4.5, 2.25, &raised));
    out.push_str(&rect(8.75, 15.75, 17.0, 37.5, 0.0, &surface));
    out.push_str(&rect(30.0, 20.0, 25.0, 14.0, 2.0, &raised));
    out.push_str(&rect(58.0, 20.0, 26.0, 14.0, 2.0, &raised));
    out.push_str(&rect(30.0, 37.0, 54.0, 13.0, 2.0, &raised));
    let text = format!(" stroke=\"{}\"", p.text);
    out.push_str(&line(34.0, 26.0, 48.0, 26.0, &text));
    out.push_str(&line(62.0, 26.0, 76.0, 26.0, &text));
    out.push_str(&line(34.0, 43.0, 66.0, 43.0, &text));
    out
}

pub(crate) fn screen() -> String {
    let mut out = rect(10.0, 6.0, 76.0, 44.0, 4.0, "");
    out.push_str(&line(40.0, 56.0, 56.0, 56.0, ""));
    out.push_str(&line(48.0, 50.0, 48.0, 56.0, ""));
    out
}

pub(crate) fn toast(x: f64, y: f64) -> String {
    let mut out = wash(x, y, 30.0, 11.0, 3.0, 0.3);
    out.push_str(&rect(x, y, 30.0, 11.0, 3.0, ""));
    out.push_str(&dot(x + 5.0, y + 5.5, 1.6));
    out.push_str(&line(x + 10.0, y + 5.5, x + 25.0, y + 5.5, ""));
    out
}

pub(crate) fn bubble(x: f64, y: f64) -> String {
    let mut out = rect(x, y, 30.0, 11.0, 5.5, "");
    out.push_str(&circle(x + 6.0, y + 5.5, 2.2, ""));
    out.push_str(&line(x + 12.0, y + 5.5, x + 25.0, y + 5.5, SOFT));
    out
}

pub(crate) fn letters(t: &str) -> String {
    let mut out = wash(30.0, 12.0, 36.0, 36.0, 8.0, 0.2);
    out.push_str(&rect(30.0, 12.0, 36.0, 36.0, 8.0, ""));
    let initial = text(
        48.0,
        35.0,
        t,
        13.0,
        "middle",
        "IBM Plex Sans, sans-serif",
        600,
    );
    out.push_str(&initial);
    out
}

pub(crate) fn mic(dx: f64) -> String {
    let mut out = rect(42.0 + dx, 8.0, 12.0, 24.0, 6.0, "");
    let cradle = format!(
        "M{} 24 Q{} 38 {} 38 Q{} 38 {} 24",
        number(36.0 + dx),
        number(36.0 + dx),
        number(48.0 + dx),
        number(60.0 + dx),
        number(60.0 + dx)
    );
    out.push_str(&path(&cradle, ""));
    out.push_str(&line(48.0 + dx, 38.0, 48.0 + dx, 48.0, ""));
    out.push_str(&line(40.0 + dx, 48.0, 56.0 + dx, 48.0, ""));
    out
}

pub(crate) fn desk_mic(dx: f64) -> String {
    let mut out = rect(34.0 + dx, 6.0, 16.0, 28.0, 8.0, "");
    for y in [14.0, 19.0, 24.0] {
        out.push_str(&line(38.0 + dx, y, 46.0 + dx, y, SOFT));
    }
    let arm = format!(
        "M{} 20 H{} V24 Q{} 38 {} 38 Q{} 38 {} 24 V20 H{}",
        number(34.0 + dx),
        number(30.0 + dx),
        number(30.0 + dx),
        number(42.0 + dx),
        number(54.0 + dx),
        number(54.0 + dx),
        number(50.0 + dx)
    );
    out.push_str(&path(&arm, ""));
    out.push_str(&line(42.0 + dx, 38.0, 42.0 + dx, 47.0, ""));
    out.push_str(&rect(31.0 + dx, 47.0, 22.0, 5.0, 2.5, ""));
    out
}

pub(crate) fn usb_logo(x: f64) -> String {
    let mut out = line(x, 16.0, x, 43.0, "");
    let arrow = format!(
        "M{} 19 L{} 13 L{} 19 Z",
        number(x - 3.5),
        number(x),
        number(x + 3.5)
    );
    out.push_str(&path(&arrow, FILL));
    out.push_str(&dot(x, 45.5, 2.8));
    let branch = format!("M{} 37 L{} 31 V27.5", number(x), number(x - 7.0));
    out.push_str(&path(&branch, ""));
    out.push_str(&circle(x - 7.0, 25.5, 2.0, ""));
    let fork = format!("M{} 32 L{} 26 V23", number(x), number(x + 7.0));
    out.push_str(&path(&fork, ""));
    out.push_str(&rect(x + 5.0, 18.5, 4.0, 4.0, 0.5, ""));
    out
}

pub(crate) fn speaker() -> String {
    let mut out = path("M22 24 H30 L42 14 V46 L30 36 H22 Z", "");
    out.push_str(&path("M50 22 Q56 30 50 38", ""));
    out.push_str(&path("M56 16 Q66 30 56 44", SOFT));
    out
}

pub(crate) fn cans() -> String {
    let mut out = path("M23.5 30 V27 Q23.5 8 48 8 Q72.5 8 72.5 27 V30", "");
    for x in [17.0, 66.0] {
        out.push_str(&wash(x, 30.0, 13.0, 22.0, 5.0, 0.2));
        out.push_str(&rect(x, 30.0, 13.0, 22.0, 5.0, ""));
    }
    out
}

pub(crate) fn bar(y: f64, fill: f64) -> String {
    let filled = 56.0 * fill;
    rect(28.0, y, 56.0, 8.0, 4.0, "") + &wash(28.0, y, filled, 8.0, 4.0, 0.55)
}

pub(crate) fn terminal(inner: &str) -> String {
    let mut out = rect(10.0, 8.0, 76.0, 44.0, 4.0, "");
    out.push_str(&line(10.0, 17.0, 86.0, 17.0, SOFT));
    for x in [16.0, 21.0, 26.0] {
        out.push_str(&dot(x, 12.5, 1.2));
    }
    out.push_str(inner);
    out
}

pub(crate) fn folder(more: &str) -> String {
    path("M12 16 H36 L40 20 H84 V48 H12 Z", more)
}

pub(crate) fn mini_folder(x: f64, y: f64) -> String {
    let d = format!(
        "M{} {} H{} L{} {} H{} V{} H{} Z",
        number(x),
        number(y),
        number(x + 4.5),
        number(x + 6.0),
        number(y + 1.5),
        number(x + 11.0),
        number(y + 9.0),
        number(x)
    );
    path(&d, "")
}

pub(crate) fn chevron(x: f64, cy: f64) -> String {
    let d = format!(
        "M{} {} L{} {} L{} {}",
        number(x),
        number(cy - 2.5),
        number(x + 2.5),
        number(cy),
        number(x),
        number(cy + 2.5)
    );
    path(&d, "")
}

pub(crate) fn chip_frame() -> String {
    let mut out = rect(30.0, 14.0, 36.0, 32.0, 4.0, "");
    for y in [22.0, 30.0, 38.0] {
        out.push_str(&line(24.0, y, 30.0, y, ""));
        out.push_str(&line(66.0, y, 72.0, y, ""));
    }
    out
}

pub(crate) fn chip(label: &str) -> String {
    chip_frame()
        + &text(
            48.0,
            33.0,
            label,
            7.0,
            "middle",
            "IBM Plex Mono, monospace",
            500,
        )
}

pub(crate) fn rune(cx: f64, cy: f64, k: f64) -> String {
    let d = format!(
        "M{} {} L{} {} L{} {} V{} L{} {} L{} {}",
        n(cx - 5.0 * k),
        n(cy - 5.0 * k),
        n(cx + 5.0 * k),
        n(cy + 5.0 * k),
        number(cx),
        n(cy + 10.0 * k),
        number(cy - 10.0 * k),
        n(cx + 5.0 * k),
        n(cy - 5.0 * k),
        n(cx - 5.0 * k),
        n(cy + 5.0 * k)
    );
    path(&d, "")
}

pub(crate) fn globe(cx: f64, cy: f64, r: f64) -> String {
    let mut out = circle(cx, cy, r, "");
    let meridian = format!(
        "M{} {} Q{} {} {} {} Q{} {} {} {}",
        number(cx),
        number(cy - r),
        number(cx + r),
        number(cy),
        number(cx),
        number(cy + r),
        number(cx - r),
        number(cy),
        number(cx),
        number(cy - r)
    );
    out.push_str(&path(&meridian, ""));
    out.push_str(&line(cx - r, cy, cx + r, cy, ""));
    out
}

pub(crate) fn moon(cx: f64, cy: f64, s: f64, more: &str) -> String {
    let x = rounded(cx + 5.3 * s);
    let y = rounded(cy - 6.0 * s);
    let d = format!(
        "M{} {} A{} {} 0 1 0 {} {} A{} {} 0 0 1 {} {} Z",
        number(x),
        number(y),
        n(7.0 * s),
        n(7.0 * s),
        number(x),
        n(y + 12.0 * s),
        n(9.0 * s),
        n(9.0 * s),
        number(x),
        number(y)
    );
    let extra = format!("{FILL}{more}");
    path(&d, &extra)
}

pub(crate) fn dial(night: bool, moon_more: &str) -> String {
    let mut out = circle(
        48.0,
        30.0,
        21.0,
        " stroke-width=\"5\" stroke-opacity=\".16\"",
    );
    for [x1, y1, x2, y2] in [
        [48.0, 13.0, 48.0, 16.0],
        [65.0, 30.0, 62.0, 30.0],
        [48.0, 47.0, 48.0, 44.0],
        [31.0, 30.0, 34.0, 30.0],
    ] {
        out.push_str(&line(x1, y1, x2, y2, SOFT));
    }
    if night {
        out.push_str(&path(
            "M29.81 19.5 A21 21 0 0 1 69 30",
            " stroke-width=\"5\"",
        ));
    }
    out.push_str(&moon(48.0, 30.0, 1.05, moon_more));
    out
}

pub(crate) fn masked(inner: &str, cutout: &str) -> String {
    let id = next_mask_id();
    format!(
        "<mask id=\"cut-{id}\" maskUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"96\" height=\"60\"><rect width=\"96\" height=\"60\" fill=\"#fff\" stroke=\"none\"/>{cutout}</mask><g mask=\"url(#cut-{id})\">{inner}</g>"
    )
}

pub(crate) fn slashed(inner: &str, x1: f64, y1: f64, x2: f64, y2: f64) -> String {
    let mut out = masked(
        inner,
        &line(x1, y1, x2, y2, " stroke=\"#000\" stroke-width=\"6.5\""),
    );
    out.push_str(&line(x1, y1, x2, y2, " stroke-width=\"2.2\""));
    out
}

pub(crate) fn copies(front: &str) -> String {
    let back = rect(28.0, 8.0, 44.0, 32.0, 3.0, SOFT);
    let cutout = rect(
        20.0,
        16.0,
        44.0,
        32.0,
        3.0,
        " fill=\"#000\" stroke=\"#000\" stroke-width=\"3\"",
    );
    let mut out = masked(&back, &cutout);
    out.push_str(&wash(20.0, 16.0, 44.0, 32.0, 3.0, 0.22));
    out.push_str(&rect(20.0, 16.0, 44.0, 32.0, 3.0, ""));
    out.push_str(front);
    out
}
