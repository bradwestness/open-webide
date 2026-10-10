//! Public component bindings and tool types. No application internals are linked.
pub mod bindings {
    wit_bindgen::generate!({path: "wit", world: "plugin", pub_export_macro: true});
}

use serde::{Deserialize, Serialize};
pub use serde_json;

/// The host independently validates names, schemas and requested capabilities.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub requires_approval: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub ok: bool,
    pub content: String,
    pub summary: String,
}

/// Plugins implement their feature behavior here.
pub trait Plugin {
    fn tools() -> Vec<Tool>;
    fn execute(name: &str, arguments: serde_json::Value) -> Result<Outcome, String>;
}

/// Call a granted general host capability. The host supplies authority.
pub fn request<T: Serialize, R: for<'de> Deserialize<'de>>(
    capability: &str,
    payload: &T,
) -> Result<R, String> {
    let input = serde_json::to_string(payload).map_err(|error| error.to_string())?;
    let response = bindings::openwebide::plugin::host::request(capability, &input)?;
    serde_json::from_str(&response).map_err(|error| error.to_string())
}

/// Export a plugin implementation using the public versioned component contract.
#[macro_export]
macro_rules! export {
    ($plugin:ty) => {
        struct OpenWebIdeComponent;
        impl $crate::bindings::Guest for OpenWebIdeComponent {
            fn tools() -> String {
                $crate::serde_json::to_string(&<$plugin as $crate::Plugin>::tools())
                    .expect("serializable tool definitions")
            }
            fn execute(name: String, arguments: String) -> Result<String, String> {
                let arguments = $crate::serde_json::from_str(&arguments)
                    .map_err(|error| error.to_string())?;
                let result = <$plugin as $crate::Plugin>::execute(&name, arguments)?;
                $crate::serde_json::to_string(&result).map_err(|error| error.to_string())
            }
        }
        $crate::bindings::export!(OpenWebIdeComponent with_types_in $crate::bindings);
    };
}
