use model::Id;

/// A navigable place in the session. Settings are not places.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
	Home,
	Channel(Id),
}

/// Session-only back and forward list. Cap 32. Empty, or `cursor < len`.
#[doc(hidden)]
#[derive(Default)]
pub struct Trail {
	places: Vec<Place>,
	cursor: usize,
}

impl Trail {
	pub(super) fn is_empty(&self) -> bool {
		self.places.is_empty()
	}

	pub(super) fn current(&self) -> Option<Place> {
		self.places.get(self.cursor).copied()
	}

	pub(super) fn visit(&mut self, place: Place) {
		if self.places.get(self.cursor) == Some(&place) {
			return;
		}
		if !self.places.is_empty() {
			self.places.truncate(self.cursor + 1);
		}
		self.places.push(place);
		if self.places.len() > 32 {
			self.places.remove(0);
		}
		self.cursor = self.places.len() - 1;
	}

	pub(super) fn peek_back(&self) -> Option<Place> {
		(self.cursor > 0).then(|| self.places[self.cursor - 1])
	}

	pub(super) fn peek_forward(&self) -> Option<Place> {
		self.places.get(self.cursor + 1).copied()
	}

	pub(super) fn commit_back(&mut self) {
		self.cursor -= 1;
	}

	pub(super) fn commit_forward(&mut self) {
		self.cursor += 1;
	}

	/// Remove the current entry. The cursor stays on what followed it, or the new end.
	pub(super) fn drop_current(&mut self) {
		if self.cursor >= self.places.len() {
			return;
		}
		self.places.remove(self.cursor);
		if self.cursor >= self.places.len() {
			self.cursor = self.places.len().saturating_sub(1);
		}
	}

	pub(super) fn drop_back(&mut self) {
		if self.cursor == 0 || self.places.is_empty() {
			return;
		}
		self.places.remove(self.cursor - 1);
		self.cursor -= 1;
	}

	pub(super) fn drop_forward(&mut self) {
		let index = self.cursor + 1;
		if index < self.places.len() {
			self.places.remove(index);
		}
	}
}
