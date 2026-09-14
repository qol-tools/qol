use super::library::{escape, split_spec};
use super::PictureContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Rest,
    Awake,
}

pub(crate) fn markup(
    spec: &str,
    tone: Tone,
    width: f32,
    height: f32,
    context: &PictureContext,
) -> Option<String> {
    let (name, argument) = split_spec(spec);
    let rest = match tone {
        Tone::Rest => " opacity=\".7\"",
        Tone::Awake => "",
    };
    match name {
        "swatch" => {
            let colour = qol_theme::accent_swatch(context.mode, argument)?;
            Some(format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"><rect width=\"{w}\" height=\"{h}\" rx=\"6\" fill=\"#{colour:06x}\"{rest}/></svg>",
                w = width,
                h = height,
            ))
        }
        "letters" => Some(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" fill=\"none\"><rect width=\"{w}\" height=\"{h}\" rx=\"6\" fill=\"currentColor\" fill-opacity=\".2\"/><rect x=\".5\" y=\".5\" width=\"{iw}\" height=\"{ih}\" rx=\"5.5\" stroke=\"currentColor\" stroke-opacity=\".8\"/><text x=\"{tx}\" y=\"{ty}\" font-size=\"16\" font-family=\"IBM Plex Sans, sans-serif\" font-weight=\"600\" text-anchor=\"middle\" fill=\"currentColor\"{rest}>{text}</text></svg>",
            w = width,
            h = height,
            iw = width - 1.0,
            ih = height - 1.0,
            tx = width / 2.0,
            ty = height / 2.0 + 5.76,
            text = escape(argument),
        )),
        _ => {
            let drawing = super::markup(spec, context)?;
            Some(match tone {
                Tone::Awake => drawing,
                Tone::Rest => rest_drawing(&drawing),
            })
        }
    }
}

pub(crate) fn is_tile(spec: &str) -> bool {
    matches!(split_spec(spec).0, "swatch" | "letters")
}

fn rest_drawing(markup: &str) -> String {
    let Some(open_end) = markup.find('>').map(|end| end + 1) else {
        return markup.to_owned();
    };
    let Some(close_start) = markup.rfind("</svg>") else {
        return markup.to_owned();
    };
    if close_start < open_end {
        return markup.to_owned();
    }
    let mut out = String::with_capacity(markup.len() + 40);
    out.push_str(&markup[..open_end]);
    out.push_str("<g opacity=\".7\">");
    out.push_str(&rest_inner(&markup[open_end..close_start]));
    out.push_str("</g>");
    out.push_str(&markup[close_start..]);
    out
}

fn rest_inner(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len() + 32);
    let mut rest = inner;
    let mut in_mask = false;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let from_tag = &rest[start..];
        let end = match from_tag.find('>') {
            Some(end) => end + 1,
            None => {
                out.push_str(from_tag);
                return out;
            }
        };
        let tag = &from_tag[..end];
        if in_mask {
            if tag.starts_with("</mask") {
                in_mask = false;
            }
            out.push_str(tag);
        } else if tag.starts_with("<mask") {
            in_mask = true;
            out.push_str(tag);
        } else {
            out.push_str(&rest_tag(tag));
        }
        rest = &from_tag[end..];
    }
    out.push_str(rest);
    out
}

fn rest_tag(tag: &str) -> String {
    let tag = rest_stroke(tag);
    rest_fill(&tag)
}

fn rest_stroke(tag: &str) -> String {
    let needle = " stroke=\"#";
    let Some(at) = tag.find(needle) else {
        return tag.to_owned();
    };
    let value_start = at + needle.len();
    let Some(offset) = tag[value_start..].find('"') else {
        return tag.to_owned();
    };
    let value_end = value_start + offset;
    format!("{} stroke=\"currentColor{}", &tag[..at], &tag[value_end..])
}

fn rest_fill(tag: &str) -> String {
    let needle = " fill=\"#";
    let Some(at) = tag.find(needle) else {
        return tag.to_owned();
    };
    let value_start = at + needle.len();
    let Some(offset) = tag[value_start..].find('"') else {
        return tag.to_owned();
    };
    let value_end = value_start + offset;
    let rewritten = format!("{} fill=\"currentColor{}", &tag[..at], &tag[value_end..]);
    set_fill_opacity(&rewritten)
}

fn set_fill_opacity(tag: &str) -> String {
    let needle = " fill-opacity=\"";
    if let Some(at) = tag.find(needle) {
        let value_start = at + needle.len();
        let Some(offset) = tag[value_start..].find('"') else {
            return tag.to_owned();
        };
        let value_end = value_start + offset;
        return format!("{} fill-opacity=\".12{}", &tag[..at], &tag[value_end..]);
    }
    let anchor = " fill=\"currentColor\"";
    match tag.find(anchor) {
        Some(at) => {
            let insert = at + anchor.len();
            format!("{} fill-opacity=\".12\"{}", &tag[..insert], &tag[insert..])
        }
        None => tag.to_owned(),
    }
}

pub(crate) fn with_default_stroke(markup: &str, width: f32) -> String {
    let Some(root_end) = markup.find('>') else {
        return markup.to_owned();
    };
    let root = &markup[..root_end];
    let Some(at) = root.find("stroke-width=\"") else {
        return markup.to_owned();
    };
    let value_start = at + "stroke-width=\"".len();
    let Some(offset) = root[value_start..].find('"') else {
        return markup.to_owned();
    };
    let value_end = value_start + offset;
    format!(
        "{}{:.4}{}{}",
        &root[..value_start],
        width,
        &root[value_end..],
        &markup[root_end..],
    )
}

pub(crate) fn fit_transform(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    width_px: u32,
    height_px: u32,
) -> (f32, f32, f32) {
    let scale = (width_px as f32 / width).min(height_px as f32 / height);
    let tx = (width_px as f32 - width * scale) / 2.0 - x * scale;
    let ty = (height_px as f32 - height * scale) / 2.0 - y * scale;
    (scale, tx, ty)
}

pub(crate) fn desaturate(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let grey = (0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32)
            .round() as u8;
        pixel[0] = grey;
        pixel[1] = grey;
        pixel[2] = grey;
    }
}

#[cfg(test)]
mod tests {
    use super::{desaturate, fit_transform, markup, with_default_stroke, Tone};
    use crate::pictures::PictureContext;
    use qol_theme::ThemeMode;

    #[test]
    fn fit_transform_keeps_the_drawing_inside_the_box() {
        let cases = [
            (2.0f32, 9.0f32, 64.0f32, 16.0f32),
            (6.0, 1.0, 10.0, 40.0),
            (4.0, 6.0, 8.0, 5.0),
        ];
        let box_width = 56.0f32;
        let box_height = 35.0f32;
        let epsilon = 0.001f32;
        for (x, y, width, height) in cases {
            let (scale, tx, ty) = fit_transform(x, y, width, height, 56, 35);
            let left = x * scale + tx;
            let right = (x + width) * scale + tx;
            let top = y * scale + ty;
            let bottom = (y + height) * scale + ty;
            assert!(
                left >= -epsilon && right <= box_width + epsilon,
                "left {left} right {right}"
            );
            assert!(
                top >= -epsilon && bottom <= box_height + epsilon,
                "top {top} bottom {bottom}"
            );
            let fills_width = (width * scale - box_width).abs() < epsilon;
            let fills_height = (height * scale - box_height).abs() < epsilon;
            assert!(fills_width || fills_height, "{x} {y} {width} {height}");
            if fills_width {
                assert!(left.abs() < epsilon, "left {left}");
                assert!((right - box_width).abs() < epsilon, "right {right}");
            }
            if fills_height {
                assert!(top.abs() < epsilon, "top {top}");
                assert!((bottom - box_height).abs() < epsilon, "bottom {bottom}");
            }
            if fills_width && !fills_height {
                assert!(((top + bottom) / 2.0 - box_height / 2.0).abs() < epsilon);
            }
            if fills_height && !fills_width {
                assert!(((left + right) / 2.0 - box_width / 2.0).abs() < epsilon);
            }
        }
    }

    #[test]
    fn rest_drawing_moves_explicit_colours_to_the_line_colour() {
        let context = PictureContext::for_accent(ThemeMode::Dark, "violet");
        let awake = markup("desktop-theme:slate", Tone::Awake, 56.0, 35.0, &context).unwrap();
        let rest = markup("desktop-theme:slate", Tone::Rest, 56.0, 35.0, &context).unwrap();
        assert!(awake.contains("stroke=\"#"));
        assert!(awake.contains("fill=\"#"));
        assert!(!rest.contains("stroke=\"#"));
        assert!(!rest.contains("fill=\"#"));
        let former_fills = awake.matches("fill=\"#").count();
        assert!(former_fills > 0);
        assert_eq!(
            rest.matches("fill=\"currentColor\" fill-opacity=\".12\"")
                .count(),
            former_fills,
        );
        assert!(rest.contains("stroke=\"none\""));
        let root_end = rest.find('>').unwrap() + 1;
        assert!(rest[root_end..].starts_with("<g opacity=\".7\">"));
        assert!(rest.ends_with("</g></svg>"));
    }

    #[test]
    fn rest_drawing_leaves_masks_alone() {
        let context = PictureContext::for_accent(ThemeMode::Dark, "violet");
        let awake = markup("headset", Tone::Awake, 56.0, 35.0, &context).unwrap();
        let rest = markup("headset", Tone::Rest, 56.0, 35.0, &context).unwrap();
        let masks = mask_elements(&awake);
        assert!(!masks.is_empty());
        assert_eq!(mask_elements(&rest), masks);
    }

    #[test]
    fn default_stroke_changes_only_the_root() {
        let context = PictureContext::for_accent(ThemeMode::Dark, "violet");
        let drawing = markup("mic-default", Tone::Awake, 56.0, 35.0, &context).unwrap();
        assert!(drawing.contains("stroke-width=\"1.5\""));
        assert_eq!(drawing.matches("stroke-width=\"2.42\"").count(), 1);
        let changed = with_default_stroke(&drawing, 2.0);
        assert!(changed.contains("stroke-width=\"2.0000\""));
        assert!(!changed.contains("stroke-width=\"1.5\""));
        assert_eq!(changed.matches("stroke-width=\"2.42\"").count(), 1);
    }

    #[test]
    fn desaturate_keeps_alpha_and_equalises_channels() {
        let mut pixels = [10u8, 200, 30, 128, 0, 0, 0, 0, 255, 255, 255, 255];
        desaturate(&mut pixels);
        assert_eq!(pixels[0], 147);
        assert_eq!(pixels[0], pixels[1]);
        assert_eq!(pixels[1], pixels[2]);
        assert_eq!(pixels[3], 128);
        assert_eq!(pixels[7], 0);
        assert_eq!(pixels[11], 255);
    }

    fn mask_elements(markup: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut rest = markup;
        while let Some(start) = rest.find("<mask") {
            let Some(offset) = rest[start..].find("</mask>") else {
                break;
            };
            let end = start + offset + "</mask>".len();
            out.push(&rest[start..end]);
            rest = &rest[end..];
        }
        out
    }
}
