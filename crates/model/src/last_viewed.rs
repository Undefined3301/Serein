use crate::Id;

/// Account-local last selected channel per guild. Not synchronized to Discord.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct LastViewedChannels {
	pub pairs: Vec<(Id, Id)>,
}

impl LastViewedChannels {
	pub const MAX_ENTRIES: usize = 1024;
	pub const MAX_JSON_BYTES: usize = 65536;

	pub fn is_valid(&self) -> bool {
		self.pairs.len() <= Self::MAX_ENTRIES
			&& self
				.pairs
				.iter()
				.enumerate()
				.all(|(index, (guild, channel))| {
					guild.0 != 0
						&& channel.0 != 0 && self.pairs[..index].iter().all(|(other, _)| other != guild)
				})
	}
}
