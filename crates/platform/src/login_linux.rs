//! A separate ephemeral GTK4/WebKit6 window for owner-operated Discord login.
use super::{
	Failure, LoginDiagnostics, LoginTermination, SessionSecret, captcha::hcaptcha_origin,
	discord_origin,
};
use std::{
	cell::{Cell, RefCell},
	rc::Rc,
	sync::Arc,
	time::{Duration, Instant},
};
use webkit6::{gio, glib, prelude::*};

const LIFETIME: Duration = Duration::from_secs(600);
const QUERY_INTERVAL: Duration = Duration::from_millis(100);
const HANDLER: &str = "sereinLogin";

struct Handoff {
	opened: Instant,
	termination: Cell<Option<LoginTermination>>,
	querying: Cell<bool>,
	consumed: Cell<bool>,
	document: Cell<u64>,
	document_ready: Cell<bool>,
	last_query: Cell<Instant>,
	diagnostics: Cell<LoginDiagnostics>,
	token: RefCell<Option<SessionSecret>>,
	capability: String,
}

impl Handoff {
	fn record(&self, update: impl FnOnce(&mut LoginDiagnostics)) {
		if self.active() {
			let mut diagnostics = self.diagnostics.get();
			update(&mut diagnostics);
			self.diagnostics.set(diagnostics);
		}
	}

	fn new(capability: String) -> Self {
		let opened = Instant::now();
		Self {
			opened,
			termination: Cell::new(None),
			querying: Cell::new(false),
			consumed: Cell::new(false),
			document: Cell::new(0),
			document_ready: Cell::new(false),
			last_query: Cell::new(opened),
			diagnostics: Cell::new(LoginDiagnostics::default()),
			token: RefCell::new(None),
			capability,
		}
	}

	fn active(&self) -> bool {
		self.termination_reason().is_none()
	}

	fn termination_reason(&self) -> Option<LoginTermination> {
		if self.termination.get().is_none() && self.opened.elapsed() > LIFETIME {
			self.stop(LoginTermination::TimedOut);
		}
		self.termination.get()
	}

	fn begin_query(&self, uri: &str, now: Instant) -> bool {
		if !self.active()
			|| !self.document_ready.get()
			|| self.consumed.get()
			|| self.token.borrow().is_some()
			|| self.querying.get()
			|| now.saturating_duration_since(self.last_query.get()) < QUERY_INTERVAL
			|| !discord_origin(uri)
		{
			return false;
		}
		self.querying.set(true);
		self.last_query.set(now);
		let mut diagnostics = self.diagnostics.get();
		diagnostics.query_attempts = diagnostics.query_attempts.saturating_add(1);
		self.diagnostics.set(diagnostics);
		true
	}

	fn navigation_started(&self) {
		self.document.set(self.document.get().wrapping_add(1));
		self.document_ready.set(false);
		self.token.borrow_mut().take();
	}

	fn finish_query(
		&self,
		document: u64,
		uri: Option<&str>,
		body: Result<Option<&str>, ()>,
	) -> bool {
		self.querying.set(false);
		if body.is_err() {
			let mut diagnostics = self.diagnostics.get();
			diagnostics.query_errors = diagnostics.query_errors.saturating_add(1);
			self.diagnostics.set(diagnostics);
		}
		if document != self.document.get() {
			return false;
		}
		if let (Some(uri), Ok(Some(body))) = (uri, body) {
			self.accept(uri, body)
		} else {
			false
		}
	}

	fn accept(&self, uri: &str, body: &str) -> bool {
		if !self.active()
			|| !self.document_ready.get()
			|| self.consumed.get()
			|| self.token.borrow().is_some()
			|| !discord_origin(uri)
			|| body.len() > 2113
		{
			return false;
		}
		let Some(value) = body.strip_prefix(&self.capability) else {
			return false;
		};
		let Ok(secret) = SessionSecret::from_owner_input(value.to_owned()) else {
			return false;
		};
		self.token.replace(Some(secret));
		let mut diagnostics = self.diagnostics.get();
		diagnostics.candidate_accepted = true;
		self.diagnostics.set(diagnostics);
		true
	}

	fn take_token(&self) -> Option<SessionSecret> {
		if !self.active() || !self.document_ready.get() || self.consumed.get() {
			return None;
		}
		let token = self.token.borrow_mut().take();
		if token.is_some() {
			// Captures can be invalidated by navigation; only forwarding to Desktop is
			// irreversible. Never permit a second handoff in this login window.
			self.consumed.set(true);
		}
		token
	}

	fn stop(&self, reason: LoginTermination) {
		// An explicit cancellation always discards a candidate, including one accepted
		// earlier in this GTK pump. Our own process teardown cannot turn it into a crash.
		if reason == LoginTermination::Cancelled || self.termination.get().is_none() {
			self.termination.set(Some(reason));
		}
		self.token.borrow_mut().take();
	}
}

pub struct LoginView {
	view: webkit6::WebView,
	window: gtk4::Window,
	manager: webkit6::UserContentManager,
	state: Rc<Handoff>,
	cancel: gio::Cancellable,
	peek_script: String,
	wake: Arc<dyn Fn() + Send + Sync>,
}

impl LoginView {
	pub fn open(
		parent: Arc<winit::window::Window>,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Result<Self, Failure> {
		let login = Self::create(wake)?;
		// GTK owns its standalone window; no foreign winit/raw-handle embedding.
		let _ = parent;
		login.view.load_uri("https://discord.com/login");
		login.window.present();
		Ok(login)
	}

	fn create(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, Failure> {
		gtk4::init().map_err(|_| Failure::ProtocolAt("Linux login window unavailable"))?;
		let mut random = [0_u8; 32];
		getrandom::fill(&mut random).map_err(|_| Failure::Protocol)?;
		let capability = random
			.iter()
			.map(|byte| format!("{byte:02x}"))
			.collect::<String>()
			+ ":";
		let script = [
			include_str!("login-linux-bridge.js"),
			include_str!("login-handoff.js"),
		]
		.join("\n")
		.replace("__SEREIN_LOGIN_CAPABILITY__", &capability);
		// evaluate_javascript runs in the main frame. Restrict its result before it
		// crosses into Rust: arbitrary child-frame IPC never supplies a token body.
		let peek_script = format!(
			"(() => {{ if (window !== window.top || location.origin !== 'https://discord.com') return null; const peek = window['__serein_login_peek_{}']; if (typeof peek !== 'function') return null; const value = peek(); return typeof value === 'string' && value.length <= 2113 && /^[\\x21-\\x7e]+$/.test(value) ? value : null; }})()",
			capability.trim_end_matches(':')
		);
		let state = Rc::new(Handoff::new(capability));
		let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
		let cancel = gio::Cancellable::new();
		let session = webkit6::NetworkSession::new_ephemeral();
		session.set_persistent_credential_storage_enabled(false);
		session.set_tls_errors_policy(webkit6::TLSErrorsPolicy::Fail);
		session.connect_download_started(|_, download| download.cancel());
		let settings = webkit6::Settings::new();
		// Retain the existing REST-client browser identity. This does not establish
		// hosted-login or challenge compatibility.
		settings.set_user_agent(Some(&client_core::fingerprint::user_agent()));
		settings.set_enable_developer_extras(false);
		settings.set_enable_write_console_messages_to_stdout(false);
		settings.set_allow_file_access_from_file_urls(false);
		settings.set_allow_universal_access_from_file_urls(false);
		settings.set_allow_top_navigation_to_data_urls(false);
		settings.set_allow_modal_dialogs(false);
		settings.set_javascript_can_open_windows_automatically(false);
		settings.set_javascript_can_access_clipboard(false);
		settings.set_enable_media_stream(false);
		settings.set_enable_webrtc(false);
		// Login needs no audio/video. Avoid unnecessary GStreamer initialization on
		// machines without optional audio sinks; native voice/playback is unaffected.
		settings.set_enable_media(false);
		settings.set_enable_webaudio(false);
		// Software rendering prevents DMA-BUF/EGL initialization crashes on Wayland and in Flatpak.
		settings.set_hardware_acceleration_policy(webkit6::HardwareAccelerationPolicy::Never);
		let manager = webkit6::UserContentManager::new();
		manager.add_script(&webkit6::UserScript::new(
			&script,
			webkit6::UserContentInjectedFrames::TopFrame,
			webkit6::UserScriptInjectionTime::Start,
			&["https://discord.com/*"],
			&[],
		));
		let view = webkit6::WebView::builder()
			.network_session(&session)
			.user_content_manager(&manager)
			.settings(&settings)
			.build();
		view.set_hexpand(true);
		view.set_vexpand(true);
		let resource_state = Rc::downgrade(&state);
		view.connect_resource_load_started(move |_, resource, request| {
			let Some(state) = resource_state.upgrade() else {
				return;
			};
			if !state.active() || !request.uri().is_some_and(|uri| qr_exchange_uri(&uri)) {
				return;
			}
			// Bound diagnostic callback registration as well as retained counters.
			if state.diagnostics.get().qr_requests >= 64 {
				return;
			}
			state.record(|d| d.qr_requests += 1);
			let failed_state = Rc::downgrade(&state);
			resource.connect_failed(move |_, _| {
				if let Some(state) = failed_state.upgrade() {
					state.record(|d| {
						d.qr_network_failures = d.qr_network_failures.saturating_add(1)
					});
				}
			});
			let finished_state = Rc::downgrade(&state);
			resource.connect_finished(move |resource| {
				let (Some(state), Some(response)) = (finished_state.upgrade(), resource.response())
				else {
					return;
				};
				if !response.uri().is_some_and(|uri| qr_exchange_uri(&uri)) {
					return;
				}
				let status = response.status_code();
				if (100..=599).contains(&status) {
					state.record(|d| {
						d.qr_responses = d.qr_responses.saturating_add(1);
						d.qr_last_status = status as u16;
					});
				}
			});
		});
		let navigation_state = Rc::downgrade(&state);
		view.connect_load_changed(move |_, event| {
			if let Some(state) = navigation_state.upgrade() {
				match event {
					webkit6::LoadEvent::Started => state.navigation_started(),
					webkit6::LoadEvent::Committed => state.document_ready.set(true),
					_ => {}
				}
			}
		});
		// Resource-load signals do not identify the initiating frame. Only the bounded
		// main-frame evaluation below can supply a token body; IPC carries a wake bit.
		let weak_state = Rc::downgrade(&state);
		let weak_view = view.downgrade();
		let notify = wake.clone();
		manager.connect_script_message_received(Some(HANDLER), move |_, value| {
			let (Some(state), Some(view)) = (weak_state.upgrade(), weak_view.upgrade()) else {
				return;
			};
			if state.active()
				&& !state.consumed.get()
				&& state.token.borrow().is_none()
				&& value.is_boolean()
				&& value.to_boolean()
				&& view.uri().is_some_and(|uri| discord_origin(&uri))
			{
				let mut diagnostics = state.diagnostics.get();
				diagnostics.bridge_wakes = diagnostics.bridge_wakes.saturating_add(1);
				state.diagnostics.set(diagnostics);
				// Wake is only a hint: polling is bounded independently of frame IPC.
				if diagnostics.bridge_wakes == 1 {
					notify();
				}
			}
		});
		if !manager.register_script_message_handler(HANDLER, None) {
			return Err(Failure::ProtocolAt("Linux login bridge unavailable"));
		}
		let policy_state = Rc::downgrade(&state);
		view.connect_decide_policy(move |_, decision, kind| {
			let allowed = match kind {
				webkit6::PolicyDecisionType::NavigationAction => decision
					.downcast_ref::<webkit6::NavigationPolicyDecision>()
					.and_then(|decision| decision.navigation_action())
					.and_then(|action| action.request())
					.and_then(|request| request.uri())
					// NavigationAction includes child frames. The response policy below
					// still restricts the main document to Discord.
					.is_some_and(|uri| discord_origin(&uri) || hcaptcha_origin(&uri)),
				webkit6::PolicyDecisionType::Response => decision
					.downcast_ref::<webkit6::ResponsePolicyDecision>()
					.is_some_and(|response| {
						response.is_mime_type_supported()
							&& (!response.is_main_frame_main_resource()
								|| response
									.request()
									.and_then(|request| request.uri())
									.is_some_and(|uri| discord_origin(&uri)))
					}),
				_ => false,
			};
			if allowed {
				decision.use_();
			} else {
				if let Some(state) = policy_state.upgrade() {
					state.record(|d| {
						d.blocked_navigations = d.blocked_navigations.saturating_add(1)
					});
				}
				decision.ignore();
			}
			true
		});
		view.connect_create(|_, _| None);
		let permission_state = Rc::downgrade(&state);
		view.connect_permission_request(move |view, request| {
			// Embedded verification's cookie access is separate from device permissions.
			// This exception grants no device permissions or persistent storage.
			let verification_storage = view.uri().is_some_and(|uri| discord_origin(&uri))
				&& request
					.downcast_ref::<webkit6::WebsiteDataAccessPermissionRequest>()
					.is_some_and(|request| {
						verification_storage_domains(
							request.current_domain().as_deref(),
							request.requesting_domain().as_deref(),
						)
					});
			if request.is::<webkit6::WebsiteDataAccessPermissionRequest>()
				&& let Some(state) = permission_state.upgrade()
			{
				state.record(|d| {
					let count = if verification_storage {
						&mut d.storage_allowed
					} else {
						&mut d.storage_denied
					};
					*count = count.saturating_add(1);
				});
			}
			if verification_storage {
				request.allow();
			} else {
				request.deny();
			}
			true
		});
		view.connect_query_permission_state(|_, query| {
			query.finish(webkit6::PermissionState::Denied);
			true
		});
		view.connect_run_file_chooser(|_, request| {
			request.cancel();
			true
		});
		view.connect_authenticate(|_, request| {
			request.cancel();
			true
		});
		view.connect_context_menu(|_, _, _| true);
		view.connect_enter_fullscreen(|_| true);
		view.connect_print(|_, _| true);
		view.connect_show_notification(|_, _| true);
		let window = gtk4::Window::builder()
			.title("Discord sign-in · Serein")
			.default_width(900)
			.default_height(700)
			.child(&view)
			.build();
		let weak_window = window.downgrade();
		view.connect_close(move |_| {
			if let Some(window) = weak_window.upgrade() {
				window.close();
			}
		});
		let weak_state = Rc::downgrade(&state);
		let weak_view = view.downgrade();
		let close_cancel = cancel.clone();
		let notify = wake.clone();
		window.connect_close_request(move |_| {
			if let Some(state) = weak_state.upgrade() {
				state.stop(LoginTermination::Cancelled);
			}
			close_cancel.cancel();
			if let Some(view) = weak_view.upgrade() {
				view.stop_loading();
				view.terminate_web_process();
			}
			notify();
			glib::Propagation::Proceed
		});
		let weak_state = Rc::downgrade(&state);
		let notify = wake.clone();
		view.connect_web_process_terminated(move |_, _| {
			if let Some(state) = weak_state.upgrade() {
				state.stop(LoginTermination::WebProcessStopped);
			}
			notify();
		});
		Ok(Self {
			view,
			window,
			manager,
			state,
			cancel,
			peek_script,
			wake,
		})
	}

	pub fn token(&self) -> Option<SessionSecret> {
		self.state.take_token()
	}

	pub fn expired(&self) -> bool {
		!self.state.active()
	}

	/// The WebKit web process ended on its own (crash or kill) rather than by timeout or close.
	pub fn crashed(&self) -> bool {
		self.termination_reason() == Some(LoginTermination::WebProcessStopped)
	}

	pub fn termination_reason(&self) -> Option<LoginTermination> {
		self.state.termination_reason()
	}

	pub fn diagnostics(&self) -> LoginDiagnostics {
		let mut diagnostics = self.state.diagnostics.get();
		diagnostics.elapsed_seconds =
			self.state.opened.elapsed().as_secs().min(u16::MAX.into()) as u16;
		diagnostics.termination = self.termination_reason();
		diagnostics.webkit_version = Some((
			webkit6::functions::major_version(),
			webkit6::functions::minor_version(),
			webkit6::functions::micro_version(),
		));
		diagnostics.gtk_version = Some((
			gtk4::major_version(),
			gtk4::minor_version(),
			gtk4::micro_version(),
		));
		diagnostics
	}

	pub fn resize(&self, _parent: &winit::window::Window) {}

	pub fn pump(&self) {
		let context = glib::MainContext::default();
		let started = Instant::now();
		for _ in 0..16 {
			if started.elapsed() >= Duration::from_millis(2) || !context.pending() {
				break;
			}
			context.iteration(false);
		}
		// Query only after the main document commits, without waiting on subresources.
		// A generation check also rejects results serialized before a later navigation.
		if !self
			.view
			.uri()
			.is_some_and(|uri| self.state.begin_query(&uri, Instant::now()))
		{
			return;
		}
		let weak_state = Rc::downgrade(&self.state);
		let weak_view = self.view.downgrade();
		let notify = self.wake.clone();
		let document = self.state.document.get();
		self.view.evaluate_javascript(
			&self.peek_script,
			None,
			None,
			Some(&self.cancel),
			move |result| {
				let (Some(state), Some(view)) = (weak_state.upgrade(), weak_view.upgrade()) else {
					return;
				};
				let body = result.map(|value| {
					value
						.is_string()
						.then(|| zeroize::Zeroizing::new(String::from(value.to_str())))
				});
				if state.finish_query(
					document,
					view.uri().as_deref(),
					body.as_ref()
						.map(|body| body.as_ref().map(|body| body.as_str()))
						.map_err(|_| ()),
				) {
					notify();
				}
			},
		);
	}
}

impl Drop for LoginView {
	fn drop(&mut self) {
		self.state.stop(LoginTermination::Cancelled);
		self.cancel.cancel();
		self.manager
			.unregister_script_message_handler(HANDLER, None);
		self.manager.remove_all_scripts();
		self.view.stop_loading();
		self.view.terminate_web_process();
		self.window.set_child(None::<&gtk4::Widget>);
		self.window.destroy();
	}
}

fn verification_storage_domains(current: Option<&str>, requesting: Option<&str>) -> bool {
	current == Some("discord.com")
		&& requesting
			.is_some_and(|domain| domain == "hcaptcha.com" || domain.ends_with(".hcaptcha.com"))
}

fn qr_exchange_uri(value: &str) -> bool {
	if value.len() > 2048 || !discord_origin(value) {
		return false;
	}
	let Ok(url) = url::Url::parse(value) else {
		return false;
	};
	let Some(path) = url.path().strip_prefix("/api/v") else {
		return false;
	};
	let Some((version, route)) = path.split_once('/') else {
		return false;
	};
	!version.is_empty()
		&& version.bytes().all(|b| b.is_ascii_digit())
		&& route == "users/@me/remote-auth/login"
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn qr_diagnostics_scope_and_cancellation() {
		assert!(qr_exchange_uri(
			"https://discord.com/api/v9/users/@me/remote-auth/login"
		));
		assert!(qr_exchange_uri(
			"https://discord.com/api/v10/users/@me/remote-auth/login?ignored=1"
		));
		for uri in [
			"http://discord.com/api/v9/users/@me/remote-auth/login",
			"https://discord.com.evil.test/api/v9/users/@me/remote-auth/login",
			"https://discord.com:444/api/v9/users/@me/remote-auth/login",
			"https://user@discord.com/api/v9/users/@me/remote-auth/login",
			"https://discord.com/api/vx/users/@me/remote-auth/login",
			"https://discord.com/api/v9/users/@me",
		] {
			assert!(!qr_exchange_uri(uri));
		}
		assert!(!qr_exchange_uri(&format!(
			"https://discord.com/api/v9/users/@me/remote-auth/login?{}",
			"x".repeat(2048)
		)));
		let state = Handoff::new("synthetic".into());
		state.record(|d| d.qr_last_status = 400);
		assert_eq!(state.diagnostics.get().qr_last_status, 400);
		state.stop(LoginTermination::Cancelled);
		state.record(|d| d.qr_last_status = 200);
		assert_eq!(state.diagnostics.get().qr_last_status, 400);
	}

	#[test]
	fn verification_storage_is_limited_to_hcaptcha_embedded_in_discord() {
		for domain in ["hcaptcha.com", "newassets.hcaptcha.com"] {
			assert!(verification_storage_domains(
				Some("discord.com"),
				Some(domain)
			));
		}
		for domain in [
			None,
			Some("evil.test"),
			Some("hcaptcha.com.evil.test"),
			Some("evilhcaptcha.com"),
		] {
			assert!(!verification_storage_domains(Some("discord.com"), domain));
		}
		for domain in [None, Some("evil.test"), Some("discord.com.evil.test")] {
			assert!(!verification_storage_domains(domain, Some("hcaptcha.com")));
		}
	}

	#[test]
	fn handoff_is_scoped_bounded_single_use_and_closed_before_late_results() {
		let state = Handoff::new("a".repeat(64) + ":");
		state.document_ready.set(true);
		let valid = state.capability.clone() + &"T".repeat(2048);
		assert_eq!(valid.len(), 2113);
		assert!(!state.accept("https://evil.test/", &valid));
		assert!(!state.accept(
			"https://discord.com/login",
			&("b".repeat(64) + ":synthetic-token-value")
		));
		assert!(!state.accept("https://discord.com/login", &(valid.clone() + "T")));
		assert!(!state.accept(
			"https://discord.com/login",
			&(state.capability.clone() + "invalid token value")
		));
		assert!(state.accept("https://discord.com/login", &valid));
		assert_eq!(state.token.borrow().as_ref().unwrap().expose().len(), 2048);
		assert!(state.take_token().is_some());
		assert!(state.take_token().is_none());
		assert!(!state.accept("https://discord.com/login", &valid));
		state.querying.set(true);
		state.stop(LoginTermination::Cancelled);
		assert!(state.token.borrow().is_none());
		assert!(!state.finish_query(0, Some("https://discord.com/login"), Ok(Some(&valid))));
		assert!(!state.querying.get());
	}

	#[test]
	fn handoff_queries_retry_without_wakes_and_stay_bounded() {
		let state = Handoff::new("a".repeat(64) + ":");
		state.document_ready.set(true);
		let uri = "https://discord.com/login";
		let now = state.last_query.get();
		assert!(!state.begin_query(uri, now));
		assert!(!state.begin_query("https://evil.test/", now + QUERY_INTERVAL));
		assert!(state.begin_query(uri, now + QUERY_INTERVAL));
		assert!(!state.begin_query(uri, now + QUERY_INTERVAL * 2));
		assert!(!state.finish_query(0, Some(uri), Err(())));
		assert!(!state.begin_query(uri, now + QUERY_INTERVAL + Duration::from_millis(99)));
		assert!(state.begin_query(uri, now + QUERY_INTERVAL * 2));
		assert!(!state.finish_query(0, Some(uri), Ok(None)));
		assert!(state.begin_query(uri, now + QUERY_INTERVAL * 3));
		let body = state.capability.clone() + "synthetic-login-candidate";
		assert!(state.finish_query(0, Some(uri), Ok(Some(&body))));
		assert!(!state.begin_query(uri, now + QUERY_INTERVAL * 4));
		let diagnostics = state.diagnostics.get();
		assert_eq!(diagnostics.bridge_wakes, 0);
		assert_eq!(diagnostics.query_attempts, 3);
		assert_eq!(diagnostics.query_errors, 1);
		assert!(diagnostics.candidate_accepted);

		let state = Handoff::new("b".repeat(64) + ":");
		state.document_ready.set(true);
		let mut diagnostics = state.diagnostics.get();
		diagnostics.query_attempts = u16::MAX;
		diagnostics.query_errors = u16::MAX;
		state.diagnostics.set(diagnostics);
		assert!(state.begin_query(uri, state.last_query.get() + QUERY_INTERVAL));
		assert!(!state.finish_query(0, Some(uri), Err(())));
		assert_eq!(state.diagnostics.get().query_attempts, u16::MAX);
		assert_eq!(state.diagnostics.get().query_errors, u16::MAX);
	}

	#[test]
	fn handoff_navigation_rejects_late_results_and_clears_unconsumed_candidates() {
		let state = Handoff::new("a".repeat(64) + ":");
		state.document_ready.set(true);
		assert!(state.take_token().is_none());
		assert!(!state.consumed.get());
		let uri = "https://discord.com/login";
		let now = state.last_query.get();
		let body = state.capability.clone() + "synthetic-login-candidate";
		assert!(state.begin_query(uri, now + QUERY_INTERVAL));
		let document = state.document.get();
		state.navigation_started();
		assert!(!state.begin_query(uri, now + QUERY_INTERVAL * 2));
		assert!(!state.finish_query(document, Some(uri), Ok(Some(&body))));
		assert!(!state.querying.get());
		assert!(!state.diagnostics.get().candidate_accepted);
		assert!(!state.begin_query(uri, now + QUERY_INTERVAL * 2));
		// Committing the new main document is sufficient; no Finished event is needed.
		state.document_ready.set(true);
		assert!(state.begin_query(uri, now + QUERY_INTERVAL * 2));
		assert!(state.finish_query(state.document.get(), Some(uri), Ok(Some(&body))));
		assert!(state.token.borrow().is_some());
		state.navigation_started();
		assert!(state.token.borrow().is_none());
		assert!(state.diagnostics.get().candidate_accepted);
		assert!(!state.begin_query(uri, now + QUERY_INTERVAL * 3));
		assert!(!state.accept(uri, &body));
		// The old candidate was never consumed; the fresh committed document may retry.
		state.document_ready.set(true);
		assert!(state.begin_query(uri, now + QUERY_INTERVAL * 3));
		let fresh = state.capability.clone() + "synthetic-fresh-candidate";
		assert!(state.finish_query(state.document.get(), Some(uri), Ok(Some(&fresh))));
		assert!(!state.accept(uri, &body));
		let token = state.take_token().unwrap();
		assert!(token.expose() == "synthetic-fresh-candidate");
		assert!(state.take_token().is_none());
		state.navigation_started();
		state.document_ready.set(true);
		assert!(!state.begin_query(uri, now + QUERY_INTERVAL * 4));
		assert!(!state.accept(uri, &body));
		assert!(state.take_token().is_none());
	}

	#[test]
	fn handoff_cancellation_wins_and_timeout_is_distinct_from_process_stop() {
		let uri = "https://discord.com/login";
		for reason in [
			LoginTermination::Cancelled,
			LoginTermination::WebProcessStopped,
		] {
			let state = Handoff::new("a".repeat(64) + ":");
			state.document_ready.set(true);
			let body = state.capability.clone() + "synthetic-login-candidate";
			assert!(state.accept(uri, &body));
			state.stop(reason);
			assert_eq!(state.termination_reason(), Some(reason));
			assert!(state.token.borrow().is_none());
			state.stop(LoginTermination::Cancelled);
			state.stop(LoginTermination::WebProcessStopped);
			assert_eq!(
				state.termination_reason(),
				Some(LoginTermination::Cancelled)
			);
			assert!(!state.accept(uri, &body));
		}
		let mut state = Handoff::new("a".repeat(64) + ":");
		state.document_ready.set(true);
		let body = state.capability.clone() + "synthetic-login-candidate";
		assert!(state.accept(uri, &body));
		state.opened -= LIFETIME + Duration::from_secs(1);
		assert_eq!(state.termination_reason(), Some(LoginTermination::TimedOut));
		assert!(state.token.borrow().is_none());
		state.stop(LoginTermination::WebProcessStopped);
		assert_eq!(state.termination_reason(), Some(LoginTermination::TimedOut));
		assert!(!state.accept(uri, &body));
	}

	// This fixture uses WebKit's in-memory HTML loader, not a local HTTP server or
	// Discord. CSP rejects every network subresource; only synthetic strings enter it.
	fn native_fixture() -> LoginView {
		let login = LoginView::create(|| {}).expect("GTK/WebKit login fixture unavailable");
		login.view.load_html(
			"<!doctype html><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; connect-src 'none'; form-action 'none'; base-uri 'none'\"><title>Synthetic Serein login test</title>",
			Some("https://discord.com/login"),
		);
		login.window.present();
		native_until(|| {
			login.view.uri().is_some_and(|uri| discord_origin(&uri)) && !login.view.is_loading()
		});
		login
	}

	fn native_until(mut ready: impl FnMut() -> bool) {
		let context = glib::MainContext::default();
		let started = Instant::now();
		while !ready() {
			assert!(
				started.elapsed() < Duration::from_secs(10),
				"native login fixture timed out"
			);
			for _ in 0..16 {
				if !context.pending() {
					break;
				}
				context.iteration(false);
			}
			std::thread::sleep(Duration::from_millis(5));
		}
	}

	fn native_assert_script(view: &webkit6::WebView, script: &str) {
		let result = Rc::new(Cell::new(None));
		let reply = result.clone();
		view.evaluate_javascript(
			script,
			None,
			None,
			None::<&gio::Cancellable>,
			move |value| {
				reply.set(Some(
					value.is_ok_and(|value| value.is_boolean() && value.to_boolean()),
				));
			},
		);
		native_until(|| result.get().is_some());
		assert_eq!(
			result.get(),
			Some(true),
			"synthetic WebKit assertion failed"
		);
	}

	#[test]
	#[ignore = "requires an explicitly selected Linux GTK/WebKit desktop; synthetic and offline"]
	fn native_login_webkit_retry_and_lifecycle() {
		let mut login = native_fixture();
		// Remove the native wake receiver: the protected candidate must still survive.
		login
			.manager
			.unregister_script_message_handler(HANDLER, None);
		native_assert_script(
			&login.view,
			// Real XHR interception, without send(): this cannot start a network request.
			"(() => { const request = new XMLHttpRequest(); const opened = request.open('GET', 'https://discord.com/api/v9/users/@me'); const header = request.setRequestHeader('Authorization', 'synthetic-login-candidate'); return opened === undefined && header === undefined; })()",
		);
		native_assert_script(
			&login.view,
			&format!(
				"(() => {{ const read = () => ({}); const first = read(); return first !== null && first === read(); }})()",
				login.peek_script,
			),
		);
		let peek = std::mem::replace(
			&mut login.peek_script,
			"throw new Error('synthetic evaluation failure')".into(),
		);
		native_until(|| {
			login.pump();
			login.diagnostics().query_errors > 0
		});
		login.peek_script = peek;
		native_until(|| {
			login.pump();
			login.diagnostics().candidate_accepted
		});
		assert_eq!(login.diagnostics().bridge_wakes, 0);
		assert!(login.diagnostics().query_attempts >= 2);
		assert!(login.state.token.borrow().is_some());
		login.window.close();
		native_until(|| login.termination_reason().is_some());
		assert_eq!(
			login.termination_reason(),
			Some(LoginTermination::Cancelled)
		);
		assert!(login.token().is_none());
		assert!(!login.crashed());
		drop(login);

		let login = native_fixture();
		native_assert_script(
			&login.view,
			// CSP denies this fetch. Observe the real Promise without an unhandled rejection.
			"(() => { const result = fetch('https://discord.com/api/v9/users/@me', { headers: { Authorization: 'synthetic-login-candidate' } }); result.catch(() => {}); return result instanceof Promise && Promise.resolve(result) === result; })()",
		);
		native_until(|| {
			login.pump();
			login.state.token.borrow().is_some()
		});
		assert!(login.token().is_some());
		assert!(login.token().is_none());
		assert!(login.state.consumed.get());
		assert!(
			!login
				.state
				.begin_query("https://discord.com/login", Instant::now() + QUERY_INTERVAL,)
		);
		login.view.terminate_web_process();
		native_until(|| login.termination_reason().is_some());
		assert_eq!(
			login.termination_reason(),
			Some(LoginTermination::WebProcessStopped)
		);
		assert!(login.crashed());
		assert!(login.token().is_none());
		drop(login);

		let login = native_fixture();
		let view = login.view.clone();
		let stopped = Rc::new(Cell::new(false));
		let observed = stopped.clone();
		view.connect_web_process_terminated(move |_, _| observed.set(true));
		let state = login.state.clone();
		drop(login);
		native_until(|| stopped.get());
		assert_eq!(
			state.termination_reason(),
			Some(LoginTermination::Cancelled)
		);
		assert!(state.token.borrow().is_none());
	}
}
