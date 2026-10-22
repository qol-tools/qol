use std::ops::Range;

use gpui::{point, px, size, Bounds, Point};
use qol_gpui::scroll_list::follow_offset_y;
use qol_gpui::theme::{
    TextStyle, HEIGHT_CONTROL, HEIGHT_INLINE, HEIGHT_SETTING_ROW, SPACE_GUTTER, SPACE_INSET,
    SPACE_STACK,
};

use super::pictogram::PictogramSize;
use crate::config::ViewSize;
use crate::ui::keys::Resize;

pub(super) const ACROSS: f32 = SPACE_INSET + SPACE_STACK;
const ICONS_CELL: (f32, f32) = (120.0, 96.0);
const BIG_ICONS_CELL: (f32, f32) = (152.0, 136.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Stop {
    Compact,
    #[default]
    List,
    Roomy,
    Icons,
    BigIcons,
}

impl Stop {
    pub(super) const ALL: [Self; 5] = [
        Self::Compact,
        Self::List,
        Self::Roomy,
        Self::Icons,
        Self::BigIcons,
    ];

    pub(super) fn at(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }

    pub(super) fn index(self) -> usize {
        self as usize
    }

    pub(super) fn resized(self, resize: Resize) -> Self {
        match resize {
            Resize::Bigger => Self::at(self.index() + 1),
            Resize::Smaller => Self::at(self.index().saturating_sub(1)),
            Resize::To(index) => Self::at(index),
        }
    }

    pub(super) fn view_size(self) -> ViewSize {
        match self {
            Self::Compact => ViewSize::Compact,
            Self::List => ViewSize::List,
            Self::Roomy => ViewSize::Roomy,
            Self::Icons => ViewSize::Icons,
            Self::BigIcons => ViewSize::BigIcons,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::List => "list",
            Self::Roomy => "roomy",
            Self::Icons => "icons",
            Self::BigIcons => "big icons",
        }
    }

    pub(super) fn is_grid(self) -> bool {
        self.cell().is_some()
    }

    pub(super) fn shows_columns(self) -> bool {
        matches!(self, Self::Compact | Self::List)
    }

    pub(super) fn two_lines(self) -> bool {
        self == Self::Roomy
    }

    pub(super) fn pictogram(self) -> PictogramSize {
        match self {
            Self::Compact | Self::List | Self::Roomy => PictogramSize::Small,
            Self::Icons => PictogramSize::Medium,
            Self::BigIcons => PictogramSize::Large,
        }
    }

    pub(super) fn name_style(self) -> TextStyle {
        match self {
            Self::List | Self::Roomy => TextStyle::Name,
            Self::Compact | Self::Icons | Self::BigIcons => TextStyle::ListName,
        }
    }

    pub(super) fn cell(self) -> Option<(f32, f32)> {
        match self {
            Self::Icons => Some(ICONS_CELL),
            Self::BigIcons => Some(BIG_ICONS_CELL),
            Self::Compact | Self::List | Self::Roomy => None,
        }
    }

    fn row(self) -> f32 {
        match self {
            Self::Compact => HEIGHT_INLINE,
            Self::List => HEIGHT_CONTROL,
            Self::Roomy => HEIGHT_SETTING_ROW,
            Self::Icons => ICONS_CELL.1 + SPACE_INSET,
            Self::BigIcons => BIG_ICONS_CELL.1 + SPACE_INSET,
        }
    }
}

impl From<ViewSize> for Stop {
    fn from(size: ViewSize) -> Self {
        match size {
            ViewSize::Compact => Self::Compact,
            ViewSize::List => Self::List,
            ViewSize::Roomy => Self::Roomy,
            ViewSize::Icons => Self::Icons,
            ViewSize::BigIcons => Self::BigIcons,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Layout {
    pub(super) stop: Stop,
    pub(super) columns: usize,
    pub(super) pitch: f32,
}

impl Layout {
    pub(super) fn new(stop: Stop, width: f32) -> Self {
        let columns = stop.cell().map_or(1, |(cell, _)| {
            let room = width - 2.0 * SPACE_GUTTER + ACROSS;
            (room / (cell + ACROSS)).floor().max(1.0) as usize
        });
        Self {
            stop,
            columns,
            pitch: stop.row(),
        }
    }

    pub(super) fn line_of(self, entry: usize) -> usize {
        entry / self.columns
    }

    pub(super) fn lines(self, entries: usize) -> usize {
        entries.div_ceil(self.columns)
    }

    pub(super) fn entries(self, lines: Range<usize>, total: usize) -> Range<usize> {
        (lines.start * self.columns).min(total)..(lines.end * self.columns).min(total)
    }

    pub(super) fn top(self, entry: usize) -> f32 {
        self.line_of(entry) as f32 * self.pitch
    }

    pub(super) fn shift(self, cursor: usize, cursor_shift: isize) -> f32 {
        let was = cursor.saturating_add_signed(-cursor_shift);
        self.top(cursor) - self.top(was)
    }

    pub(super) fn max_scroll(self, total: usize, height: f32) -> f32 {
        (self.lines(total) as f32 * self.pitch - height).max(0.0)
    }

    pub(super) fn revealed(self, cursor: usize, scroll: f32, height: f32, total: usize) -> f32 {
        let viewport = Bounds::new(Point::default(), size(px(0.0), px(height)));
        let line = Bounds::new(
            point(px(0.0), px(self.top(cursor))),
            size(px(0.0), px(self.pitch)),
        );
        let followed = follow_offset_y(viewport, line, px(-scroll), px(self.pitch));
        (-f32::from(followed)).clamp(0.0, self.max_scroll(total, height))
    }
}

#[cfg(test)]
mod tests {
    use super::{Layout, Stop};
    use crate::ui::keys::Resize;

    #[test]
    fn stops_step_one_at_a_time_and_clamp_at_both_ends() {
        let bigger = [
            (Stop::Compact, Stop::List),
            (Stop::List, Stop::Roomy),
            (Stop::Roomy, Stop::Icons),
            (Stop::Icons, Stop::BigIcons),
            (Stop::BigIcons, Stop::BigIcons),
        ];
        for (from, to) in bigger {
            assert_eq!(from.resized(Resize::Bigger), to, "{from:?}");
            assert_eq!(
                to.resized(Resize::Smaller).index(),
                from.index().min(3),
                "{to:?}"
            );
        }
        assert_eq!(Stop::Compact.resized(Resize::Smaller), Stop::Compact);
        assert_eq!(Stop::default(), Stop::List);
    }

    #[test]
    fn picking_a_stop_lands_on_it_and_past_the_last_clamps() {
        for (index, stop) in Stop::ALL.into_iter().enumerate() {
            assert_eq!(Stop::Roomy.resized(Resize::To(index)), stop);
            assert_eq!(stop.index(), index);
        }
        assert_eq!(Stop::List.resized(Resize::To(9)), Stop::BigIcons);
    }

    #[test]
    fn every_stop_has_its_name_shape_and_header() {
        let cases = [
            (Stop::Compact, "compact", false, true, 28.0),
            (Stop::List, "list", false, true, 36.0),
            (Stop::Roomy, "roomy", false, false, 52.0),
            (Stop::Icons, "icons", true, false, 104.0),
            (Stop::BigIcons, "big icons", true, false, 144.0),
        ];
        for (stop, name, grid, header, pitch) in cases {
            assert_eq!(stop.name(), name);
            assert_eq!(stop.is_grid(), grid, "{stop:?}");
            assert_eq!(stop.shows_columns(), header, "{stop:?}");
            assert_eq!(Layout::new(stop, 838.0).pitch, pitch, "{stop:?}");
        }
    }

    #[test]
    fn grid_columns_fit_the_list_width() {
        let cases = [
            (Stop::List, 838.0, 1),
            (Stop::Icons, 838.0, 6),
            (Stop::BigIcons, 838.0, 4),
            (Stop::BigIcons, 840.0, 5),
            (Stop::Icons, 100.0, 1),
            (Stop::BigIcons, 0.0, 1),
        ];
        for (stop, width, columns) in cases {
            assert_eq!(
                Layout::new(stop, width).columns,
                columns,
                "{stop:?} {width}"
            );
        }
    }

    #[test]
    fn lines_cover_every_entry_and_map_back_to_their_entries() {
        let layout = Layout::new(Stop::Icons, 838.0);
        assert_eq!(layout.lines(0), 0);
        assert_eq!(layout.lines(6), 1);
        assert_eq!(layout.lines(7), 2);
        assert_eq!(layout.line_of(5), 0);
        assert_eq!(layout.line_of(6), 1);
        assert_eq!(layout.entries(1..3, 15), 6..15);
        assert_eq!(layout.entries(3..4, 15), 15..15);
        assert_eq!(layout.shift(13, 6), 104.0);
        assert_eq!(layout.shift(13, 1), 0.0);
    }

    #[test]
    fn the_cursor_stays_in_view_across_a_size_change() {
        let total = 200;
        let height = 552.0;
        let cursor = 120;
        let list = Layout::new(Stop::List, 838.0);
        let scroll = list.revealed(cursor, 0.0, height, total);
        let shown = |layout: Layout, scroll: f32| {
            let top = layout.top(cursor);
            top >= scroll && top + layout.pitch <= scroll + height
        };
        assert!(shown(list, scroll));
        for stop in Stop::ALL {
            let layout = Layout::new(stop, 838.0);
            let followed = layout.revealed(cursor, scroll, height, total);
            assert!(shown(layout, followed), "{stop:?} {followed}");
            assert!(followed <= layout.max_scroll(total, height), "{stop:?}");
        }
    }
}
