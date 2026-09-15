use super::{LocalStore, Result, StoreError};
use model::{Id, LastViewedChannels};
use rusqlite::{OptionalExtension, params};

impl LocalStore {
	pub fn last_viewed_channels(&self, account: Id) -> Result<LastViewedChannels> {
		let value: Option<Option<String>> = self
			.0
			.query_row(
				"SELECT CASE WHEN typeof(value)='text' AND length(CAST(value AS BLOB))<=65536 THEN value ELSE NULL END FROM last_viewed_channels WHERE account=?1",
				[account.to_string()],
				|row| row.get(0),
			)
			.optional()?;
		let value = match value {
			None => LastViewedChannels::default(),
			Some(None) => return Err(StoreError::Incompatible),
			Some(Some(value)) => serde_json::from_str::<LastViewedChannels>(&value)
				.map_err(|_| StoreError::Incompatible)?,
		};
		if !value.is_valid() {
			return Err(StoreError::Incompatible);
		}
		Ok(value)
	}

	pub fn save_last_viewed_channels(&self, account: Id, value: &LastViewedChannels) -> Result<()> {
		if !value.is_valid() {
			return Err(StoreError::Capacity);
		}
		if value.pairs.is_empty() {
			self.0.execute(
				"DELETE FROM last_viewed_channels WHERE account=?1",
				[account.to_string()],
			)?;
			return Ok(());
		}
		let value = serde_json::to_string(value).map_err(|_| StoreError::Incompatible)?;
		if value.len() > LastViewedChannels::MAX_JSON_BYTES {
			return Err(StoreError::Capacity);
		}
		self.0.execute(
			"INSERT INTO last_viewed_channels(account,value) VALUES(?1,?2) ON CONFLICT(account) DO UPDATE SET value=excluded.value",
			params![account.to_string(), value],
		)?;
		Ok(())
	}
}
