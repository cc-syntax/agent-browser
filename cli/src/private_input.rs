//! Fail-closed private-input session boundary. Page-side uses of field data are
//! outside this tool-owned capture/retention boundary; raw values stay in memory.
use serde_json::{json, Value};

pub const STATUS_ACTION: &str = "__agent_browser_private_input_status";
pub const REFUSED: &str = "Private input session: incompatible configuration or operation";
pub const DEBUG_REFUSED: &str = "--private-input-session is incompatible with --debug";
pub const REQUIRED: &str = "Private input requires a verified private input session";
pub const CONFIRMATION: &str = "Private input refused: confirmation required";
pub const FAILED: &str = "Private fill failed; outcome may be unknown; not retried";
pub const READ_FAILED: &str = "Private input could not be read as UTF-8";
pub const MCP_UNSUPPORTED: &str = "MCP private stdin input is intentionally unsupported";

pub fn daemon_mode() -> bool {
    std::env::var("AGENT_BROWSER_PRIVATE_INPUT_SESSION").as_deref() == Ok("1")
}

/// Unknown observer/launch environment settings fail closed, including future ones.
pub fn validate_environment() -> Result<(), &'static str> {
    for (key, value) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if key.starts_with("AGENT_BROWSER_") {
            let allowed = matches!(
                key.as_ref(),
                "AGENT_BROWSER_PRIVATE_INPUT_SESSION"
                    | "AGENT_BROWSER_DAEMON"
                    | "AGENT_BROWSER_SESSION"
                    | "AGENT_BROWSER_SOCKET_DIR"
                    | "AGENT_BROWSER_JSON"
                    | "AGENT_BROWSER_IDLE_TIMEOUT_MS"
                    | "AGENT_BROWSER_DEFAULT_TIMEOUT"
                    | "AGENT_BROWSER_ALLOWED_DOMAINS"
                    | "AGENT_BROWSER_ACTION_POLICY"
                    | "AGENT_BROWSER_CONFIRM_ACTIONS"
                    | "AGENT_BROWSER_HIDE_SCROLLBARS"
                    | "AGENT_BROWSER_NO_XVFB"
                    | "AGENT_BROWSER_NO_AUTO_DIALOG"
            ) || (key == "AGENT_BROWSER_PLUGINS" && value == "[]")
                || (key == "AGENT_BROWSER_ENGINE" && value == "chrome")
                || (key == "AGENT_BROWSER_NO_WEBMCP" && value == "1");
            if !allowed {
                return Err(if key == "AGENT_BROWSER_DEBUG" {
                    DEBUG_REFUSED
                } else {
                    REFUSED
                });
            }
        }
        if matches!(
            key.as_ref(),
            "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "ALL_PROXY"
                | "http_proxy"
                | "https_proxy"
                | "all_proxy"
                | "NO_PROXY"
                | "no_proxy"
        ) && !value.is_empty()
        {
            return Err(REFUSED);
        }
    }
    Ok(())
}

pub fn validate_config(config: &Value) -> Result<(), &'static str> {
    let object = config.as_object().ok_or(REFUSED)?;
    for (key, value) in object {
        if key == "debug" && !value.is_null() {
            return Err(DEBUG_REFUSED);
        }
        if !value.is_null()
            && !matches!(
                key.as_str(),
                "privateInputSession"
                    | "json"
                    | "session"
                    | "idleTimeout"
                    | "allowedDomains"
                    | "actionPolicy"
                    | "confirmActions"
                    | "hideScrollbars"
                    | "noAutoDialog"
            )
        {
            return Err(REFUSED);
        }
    }
    Ok(())
}

pub fn validate_cli_args(args: &[String]) -> Result<(), &'static str> {
    if args.iter().any(|arg| arg == "--debug") {
        return Err(DEBUG_REFUSED);
    }
    validate_environment()?;
    for (index, arg) in args.iter().enumerate() {
        if arg == "--private-input-session"
            && args
                .get(index + 1)
                .is_some_and(|v| matches!(v.as_str(), "true" | "false"))
        {
            return Err(REFUSED);
        }
        if arg.starts_with('-')
            && !matches!(
                arg.as_str(),
                "--private-input-session"
                    | "--stdin"
                    | "--session"
                    | "--json"
                    | "--config"
                    | "--idle-timeout"
                    | "--allowed-domains"
                    | "--action-policy"
                    | "--confirm-actions"
                    | "--no-auto-dialog"
                    | "--hide-scrollbars"
                    | "--help"
                    | "-h"
                    | "--version"
                    | "-V"
            )
        {
            return Err(REFUSED);
        }
    }
    Ok(())
}

/// Side-effect-free CLI shape check before the canonical parser can read stdin/files.
/// The native operation/field allowlist remains the authority after parsing.
pub fn validate_command_tokens(args: &[String]) -> Result<(), &'static str> {
    let Some((verb, rest)) = args.split_first() else {
        return Ok(());
    };
    let allowed = match verb.as_str() {
        "back" | "forward" | "reload" | "close" | "quit" | "exit" => rest.is_empty(),
        "open" | "goto" | "navigate" | "click" | "dblclick" | "hover" | "focus" | "check"
        | "uncheck" | "press" | "key" | "scrollintoview" | "scrollinto" => rest.len() == 1,
        "fill" => rest.len() == 2 && rest[1] == "--stdin",
        "select" => rest.len() >= 2,
        "scroll" => rest.len() <= 2,
        "get" => match rest.first().map(String::as_str) {
            Some("url" | "title") => rest.len() == 1,
            Some("text") => rest.len() == 2,
            _ => false,
        },
        "is" => rest.len() == 2 && matches!(rest[0].as_str(), "visible" | "enabled" | "checked"),
        "dialog" => rest.len() == 1 && matches!(rest[0].as_str(), "accept" | "dismiss" | "status"),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(REFUSED)
    }
}

/// Inspect command tokens and nested batch commands, never arbitrary page/script text.
pub fn contains_private_args(args: &[String]) -> bool {
    let (clean, private) = crate::flags::clean_args_with_private_flag(args);
    if private
        || (clean.first().map(String::as_str) == Some("fill")
            && clean[1..].iter().any(|arg| arg == "--stdin"))
    {
        return true;
    }
    clean.first().map(String::as_str) == Some("batch")
        && clean[1..]
            .iter()
            .filter(|arg| arg.as_str() != "--bail")
            .any(|command| contains_private_args(&crate::commands::shell_words_split(command)))
}

pub fn contains_private_batch(body: &str) -> bool {
    serde_json::from_str::<Vec<Vec<String>>>(body)
        .is_ok_and(|commands| commands.iter().any(|args| contains_private_args(args)))
}

/// Fixed privacy failures share the canonical CLI JSON envelope.
pub fn exit_error(reason: &str, json: bool) -> ! {
    if json {
        crate::print_json_error(reason);
    } else {
        eprintln!("{}", reason);
    }
    std::process::exit(1)
}

/// Immutable session state; no setter and no serialization/persistence support.
pub struct PrivateInput {
    enabled: bool,
    values: Vec<String>,
}
impl PrivateInput {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            values: Vec::new(),
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn guard(&self, cmd: &Value) -> Result<(), &'static str> {
        let action = cmd.get("action").and_then(Value::as_str).ok_or(REFUSED)?;
        let private_fill = cmd.get("privateInput").and_then(Value::as_bool) == Some(true);
        if private_fill && (!self.enabled || action != "fill") {
            return Err(REQUIRED);
        }
        if let Some(mode) = cmd.get("privateInputSession") {
            if mode.as_bool() != Some(self.enabled) {
                return Err(REQUIRED);
            }
        }
        if !self.enabled {
            return Ok(());
        }
        let fields: &[&str] = match action {
            STATUS_ACTION
            | "close"
            | "__agent_browser_internal_shutdown"
            | "url"
            | "title"
            | "back"
            | "forward"
            | "reload" => &[],
            "navigate" => &["url", "waitUntil"],
            "fill" => &["selector", "value", "privateInput", "stdin"],
            "click" | "dblclick" | "hover" | "focus" | "clear" | "check" | "uncheck"
            | "isvisible" | "isenabled" | "ischecked" | "gettext" | "scrollintoview" => {
                &["selector"]
            }
            "press" => &["key", "selector"],
            "scroll" => &["direction", "amount", "selector"],
            "select" => &["selector", "values"],
            "dialog" => &["response", "accept", "text"],
            // Snapshot/raw extraction and value/attribute reads are intentionally
            // unsupported until protected-value masking is verified end to end.
            _ => return Err(REFUSED),
        };
        if action == "fill" && !private_fill {
            return Err(REQUIRED);
        }
        let object = cmd.as_object().ok_or(REFUSED)?;
        for (key, value) in object {
            let benign_internal = (key == "plugins" && value == &json!([]))
                || (key == "pinTab" && value == &json!(false));
            if !benign_internal
                && !matches!(key.as_str(), "id" | "action" | "privateInputSession")
                && !fields.contains(&key.as_str())
            {
                return Err(REFUSED);
            }
        }
        Ok(())
    }
    pub fn remember(&mut self, cmd: &Value) {
        if self.enabled && cmd.get("action").and_then(Value::as_str) == Some("fill") {
            if let Some(value) = cmd.get("value").and_then(Value::as_str) {
                if !value.is_empty() && !self.values.iter().any(|v| v == value) {
                    self.values.push(value.to_string());
                    self.values.sort_by_key(|v| std::cmp::Reverse(v.len()));
                }
            }
        }
    }
    /// Preserve trusted protocol structure; only page/error/warning payloads are redacted.
    pub fn redact_response(&self, response: &mut Value, action: &str) {
        if !self.enabled {
            return;
        }
        if let Some(object) = response.as_object_mut() {
            for (key, value) in object {
                match key.as_str() {
                    "id" | "success" | "code" => {}
                    "data" => {
                        if let Some(data) = value.as_object_mut() {
                            let old = std::mem::take(data);
                            for (key, mut value) in old {
                                if matches!(
                                    (action, key.as_str()),
                                    (
                                        STATUS_ACTION,
                                        "contract" | "privateInputSession" | "disabled"
                                    ) | ("fill", "filled")
                                        | ("close", "closed" | "restoreStatus" | "saveStatus")
                                ) || key == "lifecycle"
                                {
                                    // Fixed native lifecycle, never page data.
                                    data.insert(key, value);
                                } else {
                                    // These keys are constructed by the permitted native handlers;
                                    // their contents (including nested page keys) remain untrusted.
                                    let trusted_key = matches!(
                                        key.as_str(),
                                        "url"
                                            | "title"
                                            | "text"
                                            | "origin"
                                            | "status"
                                            | "clicked"
                                            | "newTab"
                                            | "dialogOpened"
                                            | "pressed"
                                            | "hovered"
                                            | "scrolled"
                                            | "selected"
                                            | "checked"
                                            | "unchecked"
                                            | "visible"
                                            | "enabled"
                                            | "focused"
                                            | "cleared"
                                            | "hasDialog"
                                            | "type"
                                            | "message"
                                            | "defaultPrompt"
                                            | "handled"
                                            | "accepted"
                                    );
                                    let mut key = Value::String(key);
                                    if !trusted_key {
                                        self.redact(&mut key);
                                    }
                                    self.redact(&mut value);
                                    data.insert(key.as_str().unwrap().to_string(), value);
                                }
                            }
                        } else {
                            self.redact(value);
                        }
                    }
                    _ => self.redact(value),
                }
            }
        } else {
            self.redact(response);
        }
    }

    /// Redact untrusted keys as well as values: pages can reflect into either.
    fn redact(&self, response: &mut Value) {
        if !self.enabled {
            return;
        }
        match response {
            Value::String(text) => {
                for value in &self.values {
                    *text = text.replace(value, "[private]");
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.redact(item);
                }
            }
            Value::Object(object) => {
                let old = std::mem::take(object);
                for (key, mut value) in old {
                    let mut key = Value::String(key);
                    self.redact(&mut key);
                    self.redact(&mut value);
                    object.insert(key.as_str().unwrap().to_string(), value);
                }
            }
            _ => {}
        }
    }
}

pub fn refusal(message: &str) -> Value {
    json!({"success": false, "error": message, "code": "private_input_refused"})
}
