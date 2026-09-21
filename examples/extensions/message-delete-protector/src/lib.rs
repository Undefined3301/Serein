use serein_extension_sdk::{Invocation, Output};

// Activation is a no-op. Deleted loaded messages stay in the session window without an extension.
fn activate(_input: Invocation) -> Output {
	Output::default()
}

serein_extension_sdk::export!(activate);
