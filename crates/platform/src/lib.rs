//! Credential persistence and temporary owner-operated login/verification surfaces.
pub mod badge;
pub mod captcha;
pub mod game_activity;
pub mod notifications;
pub mod save;
pub mod startup;
pub mod tray;
pub mod video;
use client_core::auth::{Failure, SessionSecret};
#[cfg(not(target_os = "linux"))]
use std::{
	cell::Cell,
	sync::{
		Arc,
		mpsc::{self, Receiver},
	},
	time::{Duration, Instant},
};
#[cfg(not(target_os = "linux"))]
use wry::{WebView, WebViewBuilder};
#[cfg(target_os = "linux")]
mod login_linux;
#[cfg(target_os = "linux")]
pub use login_linux::LoginView;

/// Logical height of the native header the desktop app draws above the login webview.
pub const LOGIN_HEADER_HEIGHT: f32 = 56.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginTermination {
	Cancelled,
	TimedOut,
	WebProcessStopped,
}

/// Allowlisted, session-only facts. Never store page text, URLs or credentials here.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoginDiagnostics {
	pub elapsed_seconds: u16,
	pub bridge_wakes: u16,
	pub query_attempts: u16,
	pub query_errors: u16,
	pub candidate_accepted: bool,
	pub termination: Option<LoginTermination>,
	pub webkit_version: Option<(u32, u32, u32)>,
	pub gtk_version: Option<(u32, u32, u32)>,
}
impl LoginDiagnostics {
	/// Created only for an explicit copy action; fixed fields keep the report under 4 KiB.
	pub fn summary(&self) -> String {
		let display = if cfg!(target_os = "linux") {
			if std::env::var_os("WAYLAND_DISPLAY").is_some() {
				"wayland"
			} else if std::env::var_os("DISPLAY").is_some() {
				"x11"
			} else {
				"unknown"
			}
		} else {
			"native"
		};
		format!(
			"Serein login diagnostics v1\napp_version={}\nos={}\ndisplay={}\nwebkit={:?}\ngtk={:?}\nelapsed_seconds={}\nbridge_wakes={}\nquery_attempts={}\nquery_errors={}\ncandidate_accepted={}\ntermination={:?}\n",
			env!("CARGO_PKG_VERSION"),
			std::env::consts::OS,
			display,
			self.webkit_version,
			self.gtk_version,
			self.elapsed_seconds,
			self.bridge_wakes,
			self.query_attempts,
			self.query_errors,
			self.candidate_accepted,
			self.termination,
		)
	}
}
const SERVICE: &str = "org.serein.desktop";
const ACCOUNT: &str = "discord-session";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialError {
	Unavailable,
	Invalid,
	TimedOut,
}
pub fn load_session() -> Result<Option<SessionSecret>, CredentialError> {
	let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|_| CredentialError::Unavailable)?;
	match entry.get_password() {
		Ok(value) => SessionSecret::from_owner_input(value)
			.map(Some)
			.map_err(|_| CredentialError::Invalid),
		Err(keyring::Error::NoEntry) => Ok(None),
		Err(_) => Err(CredentialError::Unavailable),
	}
}
pub fn save_session(secret: &SessionSecret) -> Result<(), CredentialError> {
	keyring::Entry::new(SERVICE, ACCOUNT)
		.and_then(|entry| entry.set_password(secret.expose()))
		.map_err(|_| CredentialError::Unavailable)
}
pub fn forget_session() -> Result<(), CredentialError> {
	match keyring::Entry::new(SERVICE, ACCOUNT).and_then(|entry| entry.delete_credential()) {
		Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
		Err(_) => Err(CredentialError::Unavailable),
	}
}
fn discord_origin(value: &str) -> bool {
	url::Url::parse(value).is_ok_and(|url| {
		url.scheme() == "https"
			&& url.host_str() == Some("discord.com")
			&& url.port_or_known_default() == Some(443)
			&& url.username().is_empty()
			&& url.password().is_none()
	})
}
/// Receives only the account token used by THIS ephemeral, owner-operated login page.
/// No browser-profile reads, password interception, console instructions, or QR exchange implementation.
#[cfg(not(target_os = "linux"))]
pub struct LoginView {
	view: WebView,
	tokens: Receiver<SessionSecret>,
	opened: Instant,
	accepted: Cell<bool>,
}
#[cfg(not(target_os = "linux"))]
impl LoginView {
	pub fn open(
		parent: Arc<winit::window::Window>,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Result<Self, Failure> {
		let (send, tokens) = mpsc::sync_channel(1);
		let mut random = [0_u8; 32];
		getrandom::fill(&mut random).map_err(|_| Failure::Protocol)?;
		let capability = random
			.iter()
			.map(|byte| format!("{byte:02x}"))
			.collect::<String>()
			+ ":";
		let script =
			include_str!("login-handoff.js").replace("__SEREIN_LOGIN_CAPABILITY__", &capability);
		let builder = WebViewBuilder::new()
			.with_url("https://discord.com/login")
			.with_incognito(true)
			.with_devtools(false)
			.with_initialization_script_for_main_only(script, true)
			.with_navigation_handler(|url| discord_origin(&url))
			.with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
			.with_download_started_handler(|_, _| false)
			.with_ipc_handler(move |request| {
				if !discord_origin(&request.uri().to_string()) || request.body().len() > 2113 {
					return;
				}
				let body = zeroize::Zeroizing::new(request.into_body());
				let Some(value) = body.strip_prefix(&capability) else {
					return;
				};
				if let Ok(secret) = SessionSecret::from_owner_input(value.to_owned()) {
					let _ = send.try_send(secret);
					wake();
				}
			});
		let view = builder
			.with_bounds(bounds(&parent))
			.build_as_child(parent.as_ref())
			.map_err(|_| Failure::Protocol)?;
		Ok(Self {
			view,
			tokens,
			opened: Instant::now(),
			accepted: Cell::new(false),
		})
	}
	pub fn token(&self) -> Option<SessionSecret> {
		if self.expired() || self.accepted.get() {
			return None;
		}
		let token = self.tokens.try_recv().ok()?;
		self.accepted.set(true);
		Some(token)
	}
	pub fn expired(&self) -> bool {
		self.opened.elapsed() > Duration::from_secs(600)
	}
	pub fn crashed(&self) -> bool {
		false
	}
	pub fn termination_reason(&self) -> Option<LoginTermination> {
		self.expired().then_some(LoginTermination::TimedOut)
	}
	pub fn diagnostics(&self) -> LoginDiagnostics {
		LoginDiagnostics {
			elapsed_seconds: self.opened.elapsed().as_secs().min(600) as u16,
			candidate_accepted: self.accepted.get(),
			termination: self.termination_reason(),
			..Default::default()
		}
	}
	pub fn resize(&self, parent: &winit::window::Window) {
		let _ = self.view.set_bounds(bounds(parent));
	}
	pub fn pump(&self) {}
}
#[cfg(not(target_os = "linux"))]
fn bounds(parent: &winit::window::Window) -> wry::Rect {
	let size = parent.inner_size();
	let header = (LOGIN_HEADER_HEIGHT as f64 * parent.scale_factor()).round() as u32;
	wry::Rect {
		position: wry::dpi::PhysicalPosition::new(0, header as i32).into(),
		size: wry::dpi::PhysicalSize::new(size.width, size.height.saturating_sub(header)).into(),
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn login_diagnostics_are_bounded_allowlisted_facts() {
		let report = LoginDiagnostics {
			elapsed_seconds: u16::MAX,
			bridge_wakes: u16::MAX,
			query_attempts: u16::MAX,
			query_errors: u16::MAX,
			candidate_accepted: true,
			termination: Some(LoginTermination::WebProcessStopped),
			webkit_version: Some((u32::MAX, u32::MAX, u32::MAX)),
			gtk_version: Some((u32::MAX, u32::MAX, u32::MAX)),
		}
		.summary();
		assert!(report.len() < 4096);
		assert_eq!(report.lines().count(), 12);
		assert!(report.contains("termination=Some(WebProcessStopped)"));
		assert!(report.lines().all(|line| line.len() < 128));
	}
	#[test]
	fn handoff_accepts_only_our_discord_origin() {
		assert!(discord_origin("https://discord.com/login"));
		for value in [
			"http://discord.com",
			"https://discord.com.evil.test",
			"https://evil.test/discord.com",
			"https://user@discord.com",
			"https://discord.com:444",
		] {
			assert!(!discord_origin(value));
		}
		let script = include_str!("login-handoff.js");
		assert!(!script.contains("localStorage"));
		assert!(!script.contains("password"));
	}
}
