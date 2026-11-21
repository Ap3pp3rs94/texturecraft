use super::{PropertySheet, ROW_HIT_HALF_HEIGHT, TEXT_SIZE, track::Track, value_text::ValueText};
use crate::theme::semibold;
use egui::{Align2, Id, Rect, Sense, Stroke, WidgetInfo, pos2};
use std::ops::RangeInclusive;
pub(super) fn shown_number(value: f32, decimals: usize) -> String {
    let shown = format!("{value:.decimals$}");
    let zero = shown.trim_start_matches('-').chars().all(|digit| matches!(digit, '0' | '.'));
    if zero { shown.trim_start_matches('-').to_owned() } else { shown }
}

/// Radius of a slider's knob
pub(super) const KNOB_RADIUS: f32 = 5.0;

/// Width of a slider's knob outline
const KNOB_STROKE: f32 = 1.5;

/// Width of a slider's track
const TRACK_WIDTH: f32 = 2.0;

/// Half the height of a signed slider's zero tick
const ZERO_TICK_HALF: f32 = 4.0;

/// The numeric value, bounds and display format shared by a slider and its text editor.
pub(super) struct SliderValue<'a> {
    value: f32,
    range: &'a RangeInclusive<f32>,
    decimals: usize,
    suffix: &'a str,
}

impl<'a> SliderValue<'a> {
    pub(super) fn new(value: f32, range: &'a RangeInclusive<f32>, decimals: usize, suffix: &'a str) -> Self {
        Self { value, range, decimals, suffix }
    }

    fn shown(&self) -> String {
        format!("{}{}", shown_number(self.value, self.decimals), self.suffix)
    }
}

impl PropertySheet<'_> {
    pub(crate) fn slider(&mut self, label: &str, value: &mut f32, range: RangeInclusive<f32>, decimals: usize) -> Option<bool> {
        self.slider_with_suffix(label, value, range, decimals, "")
    }

    pub(crate) fn slider_with_suffix(&mut self, label: &str, value: &mut f32, range: RangeInclusive<f32>, decimals: usize, suffix: &str) -> Option<bool> {
        let (control, center_y) = self.row(label);
        let id = self.next_id();
        let track = Track { start: control.left() + KNOB_RADIUS, end: self.track_end(), center_y, zero: None, ramp: false };
        let dragged = self.track(id, label, &track, value, &range);
        let typed = self.value_field(id, label, center_y, SliderValue::new(*value, &range, decimals, suffix));
        Self::apply_typed(value, typed, dragged)
    }

    pub(crate) fn signed_slider(&mut self, label: &str, value: &mut f32, range: RangeInclusive<f32>, decimals: usize) -> Option<bool> {
        let (control, center_y) = self.row(label);
        let id = self.next_id();
        let track = Track { start: control.left() + KNOB_RADIUS, end: self.track_end(), center_y, zero: Some(0.0), ramp: false };
        let dragged = self.track(id, label, &track, value, &range);
        let typed = self.value_field(id, label, center_y, SliderValue::new(*value, &range, decimals, ""));
        Self::apply_typed(value, typed, dragged)
    }

    pub(super) fn apply_typed(value: &mut f32, typed: Option<f32>, dragged: Option<bool>) -> Option<bool> {
        match typed {
            Some(typed) => {
                *value = typed;
                Some(false)
            }
            None => dragged,
        }
    }

    pub(super) fn track(&mut self, id: Id, label: impl std::fmt::Display, track: &Track, value: &mut f32, range: &RangeInclusive<f32>) -> Option<bool> {
        let (minimum, maximum) = (*range.start(), *range.end());
        let span = (maximum - minimum).max(f32::EPSILON);
        let length = (track.end - track.start).max(1.0);
        let hit = Rect::from_min_max(
            pos2(track.start - KNOB_RADIUS, track.center_y - ROW_HIT_HALF_HEIGHT),
            pos2(track.end + KNOB_RADIUS, track.center_y + ROW_HIT_HALF_HEIGHT),
        );
        let response = self.ui.interact(hit, id, Sense::click_and_drag());
        response.widget_info(|| WidgetInfo::slider(true, f64::from(*value), &label));

        let mut moved = false;
        if (response.dragged() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let picked = kept_in(minimum + (pointer.x - track.start) / length * span, range);
            if picked != *value {
                *value = picked;
                moved = true;
            }
        }

        // A press's first change starts an undo step, its later changes continue it
        let edit = crate::widgets::press_edit(self.ui, response.id, moved);

        let tokens = self.tokens;
        let at = |v: f32| track.start + ((v - minimum) / span).clamp(0.0, 1.0) * length;
        let knob_x = at(*value);
        let zero_x = track.zero.map_or(track.start, |zero| at(kept_in(zero, range)));
        let painter = self.ui.painter();
        if !track.ramp {
            painter.line_segment([pos2(track.start, track.center_y), pos2(track.end, track.center_y)], Stroke::new(TRACK_WIDTH, tokens.button_border));
            painter.line_segment([pos2(zero_x.min(knob_x), track.center_y), pos2(zero_x.max(knob_x), track.center_y)], Stroke::new(TRACK_WIDTH, tokens.accent));
        }
        if track.zero.is_some() {
            let tick = [pos2(zero_x, track.center_y - ZERO_TICK_HALF), pos2(zero_x, track.center_y + ZERO_TICK_HALF)];
            painter.line_segment(tick, Stroke::new(1.0, tokens.text_dim));
        }
        let outline = if response.hovered() || response.dragged() { tokens.accent } else { tokens.text };
        painter.circle(pos2(knob_x, track.center_y), KNOB_RADIUS, tokens.panel, Stroke::new(KNOB_STROKE, outline));
        edit
    }

    pub(super) fn value_field(&mut self, id: Id, label: impl std::fmt::Display, center_y: f32, field: SliderValue<'_>) -> Option<f32> {
        let shown = field.shown();
        self.column_value_field(id, &label, center_y, &shown, field)
    }

    pub(super) fn column_value_field(&mut self, id: Id, label: &dyn std::fmt::Display, center_y: f32, shown: &str, field: SliderValue<'_>) -> Option<f32> {
        self.measure_value(shown);
        let width = self.used.value_width.max(super::VALUE_MIN_WIDTH);
        self.value_text_field(id, ValueText { label, right: self.right, width, center_y, shown }, field)
    }

    fn value_text_field(&mut self, id: Id, value_text: ValueText<'_>, field: SliderValue<'_>) -> Option<f32> {
        let ValueText { label, right, width, center_y, shown } = value_text;
        let SliderValue { value, range, decimals, suffix } = field;
        let rect = Rect::from_min_max(pos2(right - width, center_y - ROW_HIT_HALF_HEIGHT), pos2(right, center_y + ROW_HIT_HALF_HEIGHT));
        let field_id = id.with((self.id, "typed"));
        let editing = self.typing_draft(field_id);
        let seeded = || shown_number(value, decimals);

        let tokens = self.tokens;
        let Some(mut draft) = editing else {
            self.skip_typing_edit();
            let response = self.ui.interact(rect, id.with("value"), Sense::click()).on_hover_cursor(egui::CursorIcon::Text);
            response.widget_info(|| WidgetInfo::labeled(egui::Role::Label, true, crate::i18n::trf("{label} value", &[("label", label)])));
            self.ui.painter().text(pos2(right, center_y), Align2::RIGHT_CENTER, shown, semibold(TEXT_SIZE), tokens.text);

            if response.hovered() {
                self.value_line(rect.left(), right, center_y, tokens.field_border);
            }
            if response.clicked() {
                self.start_typing(field_id, seeded());
            }
            return None;
        };

        // Typing happens on the value's own line: no box, the line under it goes accent
        let response = self.typing_edit(rect.left(), right, center_y, field_id, &mut draft);
        self.value_line(rect.left(), right, center_y, tokens.accent);
        if !response.lost_focus() {
            self.keep_typing(field_id, draft);
            return None;
        }

        let untouched = self.end_typing(field_id, &draft);
        let typed_text = draft.text.trim();
        if self.ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            return None;
        }

        // Leaving the field as it opened changes nothing: no undo step, no rounding to the shown decimals, no overwriting a value changed meanwhile
        if untouched || typed_text == seeded() {
            return None;
        }

        let typed: f32 = typed_text.trim_end_matches(suffix).trim_start_matches(['×', 'x']).trim().parse().ok()?;

        // A typed value that lands on the current one (a clamp, "0.750" for 0.75) is no edit either
        typed.is_finite().then(|| kept_in(typed, range)).filter(|kept| *kept != value)
    }
}

pub(super) fn kept_in(value: f32, range: &RangeInclusive<f32>) -> f32 {
    value.max(*range.start()).min(*range.end())
}
