//! Chat text selection. The block is the selection target, not the glyph.

use egui::{
	Color32, CursorIcon, Event, FontId, FullOutput, Id, LayerId, Order, PointerButton, Popup,
	PopupAnchor, Pos2, RawInput, Rect, Response, Sense, Stroke, epaint::Galley,
	text_selection::LabelSelectionState,
};
use std::sync::Arc;

/// Inline artwork positioned inside a run's galley (custom and Unicode emoji).
pub struct Artwork {
	pub rect: Rect,
	pub image: Option<egui::Image<'static>>,
	/// Shown when the image is not resolved yet.
	pub fallback: char,
}

struct Run {
	band: egui::Id,
	galley_pos: Pos2,
	galley: Arc<Galley>,
	rect: Rect,
	artwork: Vec<Artwork>,
}

struct Hole {
	id: egui::Id,
	rect: Rect,
}

/// One text block's runs, in layout order.
pub struct Surface {
	base: egui::Id,
	runs: Vec<Run>,
	/// Mentions, author, media, and other click widgets. Raised after the bands so a
	/// click still hits them and a drag still starts on the band underneath.
	holes: Vec<Hole>,
	/// Full message row, including the avatar column, header, and attachment cards.
	cover: Option<Rect>,
}

impl Surface {
	/// `salt` distinguishes several blocks under one `Ui` id (body, forwarded preview, …).
	pub fn new(ui: &egui::Ui, salt: impl egui::AsIdSalt) -> Self {
		Self {
			base: ui.id().with(salt),
			runs: Vec::new(),
			holes: Vec::new(),
			cover: None,
		}
	}

	/// Stretch the tiled bands to `rect` so a drag can start on empty chat chrome.
	pub fn cover(&mut self, rect: Rect) {
		if rect.is_positive() {
			self.cover = Some(self.cover.map_or(rect, |cover| cover.union(rect)));
		}
	}

	/// Keep `response` clickable after the tiled bands cover its rect.
	pub fn keep(&mut self, response: &Response) {
		if response.rect.is_positive() {
			self.holes.push(Hole {
				id: response.id,
				rect: response.rect,
			});
		}
	}

	/// Record a run and claim its interaction slot below later links and emoji.
	pub fn run(
		&mut self,
		ui: &egui::Ui,
		response: &Response,
		galley_pos: Pos2,
		galley: Arc<Galley>,
		artwork: Vec<Artwork>,
	) {
		let band = self.base.with(self.runs.len());
		// `Rect::NOTHING` is filtered out of this frame's hit test. Only the order index survives.
		ui.interact(Rect::NOTHING, band, Sense::click_and_drag());
		self.runs.push(Run {
			band,
			galley_pos,
			galley,
			rect: response.rect,
			artwork,
		});
	}

	/// Tile the block, register the selection, paint the text and then the artwork.
	pub fn finish(self, ui: &mut egui::Ui) {
		let block = block_rect(ui, &self.runs, self.cover);
		let mut runs = self.runs;
		if runs.is_empty() && block.is_positive() {
			runs.push(blank_run(ui, self.base, block));
		}
		tile(&mut runs, block, self.cover.is_some());
		let pointer = ui.input(|input| input.pointer.hover_pos());
		let over_hole =
			pointer.is_some_and(|pos| self.holes.iter().any(|hole| hole.rect.contains(pos)));
		let color = ui.visuals().text_color();
		for run in runs {
			if !run.rect.is_positive() || !ui.is_rect_visible(run.rect) {
				continue;
			}
			// `click_and_drag`, not `drag`: double-click word and triple-click line need click.
			let response = ui.interact(run.rect, run.band, Sense::click_and_drag());
			egui::text_selection::LabelSelectionState::label_text_selection(
				ui,
				&response,
				run.galley_pos,
				run.galley,
				color,
				Stroke::NONE,
			);
			for art in run.artwork {
				paint_artwork(ui, &art);
			}
		}
		for hole in self.holes {
			ui.interact_opt(
				hole.rect,
				hole.id,
				Sense::click(),
				egui::InteractOptions { move_to_top: true },
			);
		}
		if over_hole {
			ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
		} else if pointer.is_some_and(|pos| block.contains(pos)) {
			ui.ctx().set_cursor_icon(CursorIcon::Default);
		}
	}
}

/// Last-wins cursor and right-click handling for chat labels.
#[derive(Default)]
struct Pointer {
	menu: bool,
	silent: bool,
	cached: String,
}

impl egui::Plugin for Pointer {
	fn debug_name(&self) -> &'static str {
		"Chat selection pointer"
	}

	fn input_hook(&mut self, ctx: &egui::Context, input: &mut RawInput) {
		let selecting = ctx.plugin::<LabelSelectionState>().lock().has_selection();
		let secondary = input.events.iter().any(|event| {
			matches!(
				event,
				Event::PointerButton {
					button: PointerButton::Secondary,
					pressed: true,
					..
				}
			)
		});
		if selecting && secondary {
			input.events.retain(|event| {
				!matches!(
					event,
					Event::PointerButton {
						button: PointerButton::Secondary,
						..
					}
				)
			});
			if !input
				.events
				.iter()
				.any(|event| matches!(event, Event::Copy))
			{
				input.events.push(Event::Copy);
				self.silent = true;
			}
			self.menu = true;
		} else {
			self.menu = false;
		}
	}

	fn on_end_pass(&mut self, ui: &mut egui::Ui) {
		if !self.menu || Popup::is_any_open(ui.ctx()) {
			return;
		}
		let id = Id::unique("chat-selection-copy");
		Popup::new(
			id,
			ui.ctx().clone(),
			PopupAnchor::PointerFixed,
			LayerId::new(Order::Foreground, id),
		)
		.open_memory(Some(egui::SetOpenCommand::Bool(true)))
		.kind(egui::PopupKind::Menu)
		.show(|ui| {
			if ui.button("Copy").clicked() {
				request_copy(ui.ctx());
				ui.close();
			}
		});
	}

	fn output_hook(&mut self, ctx: &egui::Context, output: &mut FullOutput) {
		if let Some(text) = output
			.platform_output
			.commands
			.iter()
			.find_map(|command| match command {
				egui::OutputCommand::CopyText(text) => Some(text.clone()),
				_ => None,
			}) {
			self.cached = text;
			if self.silent {
				output
					.platform_output
					.commands
					.retain(|command| !matches!(command, egui::OutputCommand::CopyText(_)));
			}
			self.silent = false;
		}
		if output.platform_output.cursor_icon == CursorIcon::Text && !hovering_edit(ctx) {
			output.platform_output.cursor_icon = CursorIcon::Default;
		}
	}
}

fn hovering_edit(ctx: &egui::Context) -> bool {
	let hovered = ctx.interaction_snapshot(|snapshot| snapshot.hovered.clone());
	hovered
		.iter()
		.any(|id| egui::text_edit::TextEditState::load(ctx, *id).is_some())
}

/// Register once per context with the theme.
pub fn install(ctx: &egui::Context) {
	ctx.add_plugin(Pointer::default());
}

/// True when a label range is active.
pub fn has_selection(ctx: &egui::Context) -> bool {
	ctx.plugin::<LabelSelectionState>().lock().has_selection()
}

/// True on the frame that swallowed a right-click over a live selection.
pub fn open_menu(ctx: &egui::Context) -> bool {
	ctx.plugin_opt::<Pointer>()
		.is_some_and(|plugin| plugin.lock().menu)
}

/// Copy the text cached from the last selected range.
pub fn request_copy(ctx: &egui::Context) {
	let text = ctx
		.plugin_opt::<Pointer>()
		.map(|plugin| plugin.lock().cached.clone())
		.unwrap_or_default();
	if !text.is_empty() {
		ctx.copy_text(text);
	}
}

fn block_rect(ui: &egui::Ui, runs: &[Run], cover: Option<Rect>) -> Rect {
	let from_runs = runs.first().map(|first| {
		let pad = ui.spacing().item_spacing.y / 2.0;
		let top = runs
			.iter()
			.map(|run| run.rect.top())
			.fold(first.rect.top(), f32::min);
		let bottom = runs
			.iter()
			.map(|run| run.rect.bottom())
			.fold(first.rect.bottom(), f32::max);
		Rect::from_min_max(
			egui::pos2(ui.max_rect().left(), top - pad),
			egui::pos2(ui.max_rect().right(), bottom + pad),
		)
	});
	match (from_runs, cover) {
		(Some(runs), Some(cover)) => runs.union(cover),
		(Some(runs), None) => runs,
		(None, Some(cover)) => cover,
		(None, None) => Rect::NOTHING,
	}
}

fn tile(runs: &mut [Run], block: Rect, stitch: bool) {
	if runs.is_empty() {
		return;
	}
	let mut rows = Vec::new();
	let mut start = 0;
	let mut top = runs[0].rect.top();
	let mut bottom = runs[0].rect.bottom();
	for (index, run) in runs.iter().enumerate().skip(1) {
		let center = run.rect.center().y;
		if (top..=bottom).contains(&center) {
			top = top.min(run.rect.top());
			bottom = bottom.max(run.rect.bottom());
		} else {
			rows.push(start..index);
			start = index;
			top = run.rect.top();
			bottom = run.rect.bottom();
		}
	}
	rows.push(start..runs.len());

	let last = rows.len() - 1;
	let mut previous_bottom = block.top();
	for (row_index, range) in rows.into_iter().enumerate() {
		let natural_top = runs[range.clone()]
			.iter()
			.map(|run| run.rect.top())
			.fold(f32::INFINITY, f32::min);
		let natural_bottom = runs[range.clone()]
			.iter()
			.map(|run| run.rect.bottom())
			.fold(f32::NEG_INFINITY, f32::max);
		let row_top = if row_index == 0 {
			block.top()
		} else if stitch || natural_top <= previous_bottom + 2.0 {
			previous_bottom
		} else {
			natural_top
		};
		let row_bottom = if row_index == last {
			block.bottom()
		} else {
			natural_bottom
		};
		let first = range.start;
		let end = range.end;
		for run in &mut runs[range] {
			run.rect.min.y = row_top;
			run.rect.max.y = row_bottom;
		}
		runs[first].rect.min.x = block.left();
		runs[end - 1].rect.max.x = block.right();
		previous_bottom = row_bottom;
	}
}

fn blank_run(ui: &egui::Ui, base: egui::Id, block: Rect) -> Run {
	let galley = ui.painter().layout_no_wrap(
		String::new(),
		FontId::proportional(1.0),
		Color32::TRANSPARENT,
	);
	Run {
		band: base.with("blank"),
		galley_pos: block.min,
		galley,
		rect: block,
		artwork: Vec::new(),
	}
}

fn paint_artwork(ui: &egui::Ui, art: &Artwork) {
	if !ui.is_rect_visible(art.rect) {
		return;
	}
	let size = art.rect.width();
	if let Some(image) = &art.image {
		let painted = image.calc_size(egui::Vec2::splat(size), image.size());
		image.paint_at(ui, Rect::from_center_size(art.rect.center(), painted));
	} else {
		ui.painter().text(
			art.rect.center(),
			egui::Align2::CENTER_CENTER,
			art.fallback,
			egui::FontId::proportional(size),
			ui.visuals().weak_text_color(),
		);
	}
}
