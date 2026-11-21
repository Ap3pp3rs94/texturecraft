//! The Properties panel's rows, groups and controls, shared by every panel so they line up

mod row;
mod sheet_measure;
mod slider;
mod track;
mod value_text;
mod value_text_draft;

use egui::{Color32, CornerRadius, FontId, Id, Painter, Rect, Ui, pos2, vec2};

use crate::theme::{Tokens, semibold};

use sheet_measure::SheetMeasure;
type SheetMeasures = egui::cache::FramePublisher<Id, SheetMeasure>;

/// Side padding: every row starts and ends this far inside the panel
pub(crate) const PAD: f32 = 12.0;

/// Row pitch: every control row is one box this tall with its control centred
pub(crate) const ROW_HEIGHT: f32 = 28.0;

/// Height of dropdowns, fields, switches and buttons
const CONTROL_HEIGHT: f32 = 24.0;

/// The narrowest label column, from the panel's left padding to the controls
const LABEL_COLUMN_MIN: f32 = 90.0;

/// Labels wider than this stack above a full-width control and never widen the column
const LABEL_MAX: f32 = 132.0;

/// Air between the widest label and the controls column
const LABEL_TO_CONTROL: f32 = 12.0;

/// A label fits beside its control when it leaves at least this much air before the column
const LABEL_MIN_AIR: f32 = 8.0;

/// The narrowest value column at a slider's right end
const VALUE_MIN_WIDTH: f32 = 26.0;

/// Size of regular labels and SemiBold values
const TEXT_SIZE: f32 = 12.0;

/// The Properties panel being laid out this frame: where the controls column is, which card is open, and what this frame measured for the next one
pub(crate) struct PropertySheet<'u> {
    pub(crate) ui: &'u mut Ui,
    id: Id,
    tokens: Tokens,
    left: f32,
    right: f32,
    used: SheetMeasure,
    seen: SheetMeasure,
}

impl<'u> PropertySheet<'u> {
    /// Lays out `body` as one panel; when its measured columns change, egui reruns the frame unseen
    pub(crate) fn show<R>(ui: &'u mut Ui, id_salt: impl egui::AsIdSalt, body: impl FnOnce(&mut PropertySheet<'_>) -> R) -> R {
        let id = ui.make_persistent_id(id_salt);
        let used = ui.memory_mut(|memory| memory.caches.cache::<SheetMeasures>().get(&id).copied().unwrap_or_default());
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let rect = ui.available_rect_before_wrap();
        let mut sheet =
            PropertySheet { id, tokens: Tokens::get(ui.ctx()), left: rect.left() + PAD, right: rect.right() - PAD, used, seen: SheetMeasure::default(), ui };
        let result = body(&mut sheet);

        let seen = sheet.seen;
        sheet.ui.memory_mut(|memory| memory.caches.cache::<SheetMeasures>().set(id, seen));
        if seen != used {
            sheet.ui.ctx().request_discard("Properties columns changed");
        }
        result
    }

    /// The controls column: wide enough for this panel's labels, capped at [`LABEL_MAX`]
    fn column(&self) -> f32 {
        self.left + LABEL_COLUMN_MIN.max(self.used.label_width + LABEL_TO_CONTROL)
    }

    /// Where the knob of a slider's track ends: left of this panel's widest value
    fn track_end(&self) -> f32 {
        self.right - self.used.value_width.max(VALUE_MIN_WIDTH) - 14.0
    }

    /// Records a value's width for the panel's value column
    fn measure_value(&mut self, text: &str) {
        let width = self.text_width(text, semibold(TEXT_SIZE));
        self.seen.value_width = self.seen.value_width.max(width);
    }

    fn text_width(&self, text: &str, font: FontId) -> f32 {
        self.ui.painter().layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x
    }

    /// Takes the next `height` pixels of the panel, full width; marks what it draws down to
    fn take(&mut self, height: f32) -> Rect {
        let (rect, _) = self.ui.allocate_exact_size(vec2(self.ui.available_width(), height), egui::Sense::hover());
        rect
    }

    /// Leaves `height` pixels of air
    fn space(&mut self, height: f32) {
        self.ui.add_space(height);
    }

    fn cursor_y(&self) -> f32 {
        self.ui.cursor().top()
    }

    /// A control's box: [`CONTROL_HEIGHT`] tall, centred in its row, from `x0` to the right edge
    fn control_box(&self, x0: f32, center_y: f32) -> Rect {
        Rect::from_min_max(pos2(x0, center_y - CONTROL_HEIGHT / 2.0), pos2(self.right, center_y + CONTROL_HEIGHT / 2.0))
    }
}

const TAB_BELOW: f32 = 6.0;
const TAB_ROUNDING: u8 = 5;

pub(crate) fn full_height_tab_rect(strip: Rect, left: f32, right: f32) -> Rect {
    Rect::from_min_max(pos2(left, strip.top()), pos2(right, strip.bottom() + TAB_BELOW))
}

pub(crate) fn shown_tab_box(painter: &Painter, tokens: &Tokens, tab: Rect) {
    painter.rect_filled(tab, CornerRadius::same(TAB_ROUNDING), tokens.panel);
}

const ROW_HIT_HALF_HEIGHT: f32 = 10.0;
