//! Shared wheel-speed and hold-to-scroll autoscroll for the main lists.
use crate::design;
use egui::{AsIdSalt, IdSalt, PointerButton, Pos2, Rect, ScrollArea, Shape, Stroke, pos2};

/// Chromium / Discord default: 3 wheel lines times 40 px. winit reports one notch as `LineDelta` 1.0.
pub const DISCORD_LINE_SCROLL_SPEED: f32 = 120.0;

#[derive(Clone, Copy, Default)]
enum Drive {
	#[default]
	Idle,
	Holding {
		origin: Pos2,
		target: egui::Id,
	},
}

#[derive(Default)]
pub struct Session {
	drive: Drive,
	frame: Option<u64>,
	armed: bool,
	applied: Option<(egui::Id, f32)>,
}

impl Session {
	pub fn holding(&self) -> bool {
		matches!(self.drive, Drive::Holding { .. })
	}

	pub fn bind(&mut self, ui: &egui::Ui, target: egui::Id, area: Rect) -> f32 {
		let frame = ui.ctx().cumulative_frame_nr();
		if self.frame != Some(frame) {
			self.frame = Some(frame);
			self.armed = false;
			self.stop_if_needed(ui);
		}
		self.try_start(ui, target, area);
		match self.drive {
			Drive::Holding {
				origin,
				target: held,
			} if held == target => {
				self.armed = true;
				let pointer = ui
					.input(|input| input.pointer.hover_pos())
					.unwrap_or(origin);
				let dt = ui.input(|input| input.stable_dt);
				velocity(origin, pointer, dt)
			}
			_ => 0.0,
		}
	}

	pub fn attach(
		&mut self,
		ui: &egui::Ui,
		salt: impl AsIdSalt + Copy,
		builder: ScrollArea,
	) -> ScrollArea {
		let builder = builder.id_salt(salt);
		let target = ui.make_persistent_id(IdSalt::new(salt));
		let area = ui.available_rect_before_wrap().intersect(ui.clip_rect());
		let delta = self.bind(ui, target, area);
		if delta == 0.0 {
			if !self.holding() {
				self.applied = None;
			}
			return builder;
		}
		let Some(state) = egui::scroll_area::State::load(ui.ctx(), target) else {
			return builder;
		};
		let next = (state.offset.y - delta).max(0.0);
		if (next - state.offset.y).abs() < f32::EPSILON {
			return builder;
		}
		if let Some((id, last)) = self.applied
			&& id == target
			&& (last - state.offset.y).abs() > 0.5
			&& (next - state.offset.y).signum() == (last - state.offset.y).signum()
		{
			return builder;
		}
		self.applied = Some((target, next));
		ui.ctx().request_repaint();
		builder.vertical_scroll_offset(next)
	}

	pub fn paint(&self, ctx: &egui::Context) {
		let Drive::Holding { origin, .. } = self.drive else {
			return;
		};
		let colors = design::palette_for(ctx);
		let painter = ctx.layer_painter(egui::LayerId::new(
			egui::Order::Foreground,
			egui::Id::unique("serein-autoscroll"),
		));
		painter.circle_filled(origin, 12.0, colors.raised);
		painter.circle_stroke(origin, 12.0, Stroke::new(1.0, colors.muted));
		let tip = 3.6;
		let gap = 1.6;
		painter.add(Shape::convex_polygon(
			vec![
				pos2(origin.x, origin.y - gap - tip),
				pos2(origin.x - tip, origin.y - gap),
				pos2(origin.x + tip, origin.y - gap),
			],
			colors.text,
			Stroke::NONE,
		));
		painter.add(Shape::convex_polygon(
			vec![
				pos2(origin.x, origin.y + gap + tip),
				pos2(origin.x - tip, origin.y + gap),
				pos2(origin.x + tip, origin.y + gap),
			],
			colors.text,
			Stroke::NONE,
		));
		ctx.set_cursor_icon(egui::CursorIcon::ResizeVertical);
	}

	pub fn reap(&mut self, ctx: &egui::Context) {
		if self.frame != Some(ctx.cumulative_frame_nr()) || !self.armed {
			self.drive = Drive::Idle;
			self.applied = None;
		}
	}

	fn stop_if_needed(&mut self, ui: &egui::Ui) {
		if matches!(self.drive, Drive::Idle) {
			return;
		}
		let stop = ui.input(|input| {
			!input.focused
				|| !input.pointer.button_down(PointerButton::Middle)
				|| input.key_pressed(egui::Key::Escape)
		});
		if stop {
			self.drive = Drive::Idle;
			self.applied = None;
		}
	}

	fn try_start(&mut self, ui: &egui::Ui, target: egui::Id, area: Rect) {
		if !matches!(self.drive, Drive::Idle) {
			return;
		}
		let Some(pos) = ui.input(|input| {
			input
				.pointer
				.button_pressed(PointerButton::Middle)
				.then_some(input.pointer.hover_pos())
				.flatten()
		}) else {
			return;
		};
		if area.contains(pos) {
			self.drive = Drive::Holding {
				origin: pos,
				target,
			};
			self.armed = true;
		}
	}
}

fn velocity(origin: Pos2, pointer: Pos2, dt: f32) -> f32 {
	let distance = pointer.y - origin.y;
	let travel = (distance.abs() - 8.0).max(0.0);
	let speed = (travel * 12.0 + travel * travel * 0.12).min(12000.0);
	-distance.signum() * speed * dt.min(0.05)
}
