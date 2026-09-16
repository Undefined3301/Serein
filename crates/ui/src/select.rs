//! Chat text selection. The block is the selection target, not the glyph.

use egui::{
	Color32, CursorIcon, Event, FullOutput, Id, InteractOptions, LayerId, Order, PointerButton,
	Popup, PopupAnchor, Pos2, RawInput, Rect, Response, Sense, Stroke,
	epaint::{Galley, TextShape},
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
	painted: bool,
}

struct Hole {
	rect: Rect,
}

struct Overlay {
	id: egui::Id,
	rect: Rect,
}

/// One text block's runs, in layout order.
pub struct Surface {
	base: egui::Id,
	runs: Vec<Run>,
	/// Exclusive click widgets punched out of the bands.
	holes: Vec<Hole>,
	/// In-text emoji and links. Raised after the bands so a click still hits them.
	overlays: Vec<Overlay>,
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
			overlays: Vec::new(),
			cover: None,
		}
	}

	/// Stretch the tiled bands to `rect` so a drag can start on empty chat chrome.
	pub fn cover(&mut self, rect: Rect) {
		if rect.is_positive() {
			self.cover = Some(self.cover.map_or(rect, |cover| cover.union(rect)));
		}
	}

	/// Punch this exclusive click rect out of the tiled bands.
	pub fn keep(&mut self, response: &Response) {
		self.exclude(response.rect);
	}

	/// Punch a laid-out region that has no single widget `Response`.
	pub fn exclude(&mut self, rect: Rect) {
		if rect.is_positive() {
			self.holes.push(Hole { rect });
		}
	}

	/// Raise this in-text click after the bands. Drag still starts on the band.
	pub fn through(&mut self, response: &Response) {
		if response.rect.is_positive() {
			self.overlays.push(Overlay {
				id: response.id,
				rect: response.rect,
			});
		}
	}

	/// Record a run, paint it like a label, and claim a later band slot.
	pub fn run(
		&mut self,
		ui: &mut egui::Ui,
		response: &Response,
		galley_pos: Pos2,
		galley: Arc<Galley>,
		artwork: Vec<Artwork>,
	) {
		let band = self.base.with(self.runs.len());
		let painted =
			!artwork.is_empty() && galley.rows.iter().all(|row| row.visuals.mesh.is_empty());
		if painted {
			ui.painter().add(TextShape::new(
				galley_pos,
				galley.clone(),
				Color32::TRANSPARENT,
			));
		}
		for art in &artwork {
			paint_artwork(ui, art);
		}
		self.runs.push(Run {
			band,
			galley_pos,
			galley,
			rect: response.rect,
			painted,
		});
	}

	/// Tile the block and register selection on the remaining bands.
	pub fn finish(self, ui: &mut egui::Ui) {
		let block = block_rect(ui, &self.runs, self.cover);
		let mut runs = self.runs;
		if runs.is_empty() && block.is_positive() {
			runs.push(blank_run(ui, self.base, block));
		}
		let covered = self.cover.is_some();
		tile(&mut runs, block, covered);
		let pointer = ui.input(|input| input.pointer.hover_pos());
		let over_hole = pointer.is_some_and(|pos| {
			self.holes.iter().any(|hole| hole.rect.contains(pos))
				|| self.overlays.iter().any(|over| over.rect.contains(pos))
		});
		let menu_open = Popup::is_any_open(ui.ctx());
		let holes: Vec<Rect> = self.holes.iter().map(|hole| hole.rect).collect();
		for run in runs {
			if menu_open || !run.rect.is_positive() || !ui.is_rect_visible(run.rect) {
				continue;
			}
			let pieces = punch(run.rect, &holes);
			if pieces.is_empty() {
				continue;
			}
			let mut response: Option<Response> = None;
			for (index, piece) in pieces.iter().enumerate() {
				let id = if index == 0 {
					run.band
				} else {
					run.band.with(index)
				};
				let piece = ui.interact(*piece, id, band_sense());
				response = Some(match response.take() {
					Some(prev) => prev.union(piece),
					None => piece,
				});
			}
			let Some(response) = response else {
				continue;
			};
			if run.galley.job.text.is_empty() {
				continue;
			}
			let color = if run.painted {
				Color32::TRANSPARENT
			} else {
				ui.visuals().text_color()
			};
			egui::text_selection::LabelSelectionState::label_text_selection(
				ui,
				&response,
				run.galley_pos,
				run.galley,
				color,
				Stroke::NONE,
			);
		}
		for over in self.overlays {
			ui.interact_opt(
				over.rect,
				over.id,
				Sense::click(),
				InteractOptions { move_to_top: true },
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
		}
		self.silent = false;
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

pub(crate) fn band_sense() -> Sense {
	Sense::CLICK | Sense::DRAG
}

fn punch(rect: Rect, holes: &[Rect]) -> Vec<Rect> {
	let mut parts = vec![rect];
	for hole in holes {
		if !hole.is_positive() {
			continue;
		}
		let mut next = Vec::new();
		for part in parts {
			next.extend(subtract(part, *hole));
		}
		parts = next;
		if parts.is_empty() {
			break;
		}
	}
	parts
		.into_iter()
		.filter(|part| part.is_positive() && part.width() >= 1.0 && part.height() >= 1.0)
		.collect()
}

fn subtract(rect: Rect, hole: Rect) -> Vec<Rect> {
	let cut = rect.intersect(hole);
	if !cut.is_positive() {
		return vec![rect];
	}
	let mut parts = Vec::new();
	if rect.top() < cut.top() {
		parts.push(Rect::from_min_max(
			egui::pos2(rect.left(), rect.top()),
			egui::pos2(rect.right(), cut.top()),
		));
	}
	if cut.bottom() < rect.bottom() {
		parts.push(Rect::from_min_max(
			egui::pos2(rect.left(), cut.bottom()),
			egui::pos2(rect.right(), rect.bottom()),
		));
	}
	if rect.left() < cut.left() {
		parts.push(Rect::from_min_max(
			egui::pos2(rect.left(), cut.top()),
			egui::pos2(cut.left(), cut.bottom()),
		));
	}
	if cut.right() < rect.right() {
		parts.push(Rect::from_min_max(
			egui::pos2(cut.right(), cut.top()),
			egui::pos2(rect.right(), cut.bottom()),
		));
	}
	parts
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
		egui::FontId::proportional(1.0),
		Color32::TRANSPARENT,
	);
	Run {
		band: base.with("blank"),
		galley_pos: block.min,
		galley,
		rect: block,
		painted: true,
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
