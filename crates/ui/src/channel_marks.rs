use crate::design::Palette;
use crate::icons::{self, Icon};
use client_core::ChannelAccess;
use egui::{Color32, Pos2, Rect, Vec2};

const DIM: f32 = 0.6;
const EYE: f32 = 16.0;
const LOCK: f32 = 9.0;
const LOCK_KNOCKOUT: f32 = 5.5;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Emphasis {
	Unavailable,
	Idle,
	Unread,
	Focused,
	Connected,
}

pub(crate) fn tint(colors: &Palette, access: ChannelAccess, emphasis: Emphasis) -> Color32 {
	if access.dim() {
		return colors.muted.gamma_multiply(DIM);
	}
	match emphasis {
		Emphasis::Unavailable => colors.muted.gamma_multiply(DIM),
		Emphasis::Connected => colors.accent,
		Emphasis::Focused | Emphasis::Unread => colors.text_strong,
		Emphasis::Idle => colors.muted,
	}
}

pub(crate) fn trailing(access: ChannelAccess) -> f32 {
	if access.hidden() { EYE } else { 0.0 }
}

pub(crate) fn paint(
	painter: &egui::Painter,
	access: ChannelAccess,
	row: Rect,
	glyph: Rect,
	color: Color32,
	background: Color32,
) {
	if access.limited() {
		let lock =
			Rect::from_center_size(glyph.right_top() + Vec2::new(-1.0, 1.0), Vec2::splat(LOCK));
		painter.circle_filled(lock.center(), LOCK_KNOCKOUT, background);
		icons::paint(painter, Icon::Lock, lock, color);
	}
	if access.hidden() {
		let center = Pos2::new(row.right() - EYE * 0.5, row.center().y);
		icons::paint(
			painter,
			Icon::EyeSlash,
			Rect::from_center_size(center, Vec2::splat(EYE)),
			color,
		);
	}
}

pub(crate) fn label(access: ChannelAccess) -> &'static str {
	match (access.muted(), access.hidden(), access.limited()) {
		(false, false, false) => "",
		(true, false, false) => " · Muted",
		(false, true, false) => " · Hidden",
		(false, false, true) => " · Limited",
		(true, true, false) => " · Muted · Hidden",
		(true, false, true) => " · Muted · Limited",
		(false, true, true) => " · Hidden · Limited",
		(true, true, true) => " · Muted · Hidden · Limited",
	}
}
