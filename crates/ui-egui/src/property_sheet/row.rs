use super::{LABEL_MAX, LABEL_MIN_AIR, PropertySheet, ROW_HEIGHT, TEXT_SIZE};
use egui::{Align2, FontId, Id, Rect, pos2};
const STACK_AIR: f32 = 4.0;
const STACK_LABEL_LINE: f32 = 14.0;
impl PropertySheet<'_> {
    pub(crate) fn row(&mut self, label: &str) -> (Rect, f32) {
        let x0 = self.lead(label, false);
        let rect = self.take(ROW_HEIGHT);
        let center_y = rect.center().y;

        (self.control_box(x0, center_y), center_y)
    }
    pub(crate) fn lead(&mut self, label: &str, stack: bool) -> f32 {
        let font = FontId::proportional(TEXT_SIZE);
        let width = self.text_width(label, font.clone());
        if width <= LABEL_MAX {
            self.seen.label_width = self.seen.label_width.max(width);
        }

        let text = self.tokens.text;
        if width <= self.column() - self.left - LABEL_MIN_AIR && !stack {
            let center_y = self.cursor_y() + ROW_HEIGHT / 2.0;
            self.ui.painter().text(pos2(self.left, center_y), Align2::LEFT_CENTER, label, font, text);
            return self.column();
        }

        self.space(STACK_AIR);
        let rect = self.take(STACK_LABEL_LINE);
        self.ui.painter().text(pos2(self.left, rect.top() + 8.0), Align2::LEFT_CENTER, label, font, text);
        self.left
    }
    pub(super) fn next_id(&mut self) -> Id {
        let id = self.ui.next_auto_id();
        self.ui.skip_ahead_auto_ids(1);
        id
    }
}
