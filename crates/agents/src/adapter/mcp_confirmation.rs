use serde_json::{Value, json};

pub(super) struct Confirmation {
    pub prompt: String,
    request: Value,
}

impl Confirmation {
    pub fn parse(server: &str, request: &Value) -> Result<Self, &'static str> {
        let message = request["message"].as_str().unwrap_or_default();
        let mode = request["mode"].as_str().unwrap_or("form");
        let schema = request
            .get("requestedSchema")
            .or_else(|| request.get("requested_schema"))
            .unwrap_or(&Value::Null);
        let Some(schema) = schema.as_object() else {
            return Err("MCP confirmation is missing its form schema");
        };
        // A confirmation has no values to collect. Never treat an unsupported form
        // or a URL authorization flow as permission to submit an empty object.
        if mode != "form"
            || server.trim().is_empty()
            || message.trim().is_empty()
            || schema.get("type").and_then(Value::as_str) != Some("object")
            || !schema.iter().all(|(key, value)| match key.as_str() {
                "properties" => value
                    .as_object()
                    .is_some_and(|properties| properties.is_empty()),
                "required" => value.as_array().is_some_and(|required| required.is_empty()),
                "additionalProperties" => value.is_boolean(),
                "type" | "title" | "description" | "$schema" => true,
                _ => false,
            })
        {
            return Err("Only MCP confirmation forms without input fields are supported");
        }
        let metadata = &request["_meta"];
        let requester = metadata["connector_name"].as_str().unwrap_or(server);
        let mut prompt = format!("{message}\nRequested by: {requester}");
        for (key, label) in [
            ("subtitle", "Note"),
            ("tool_name", "Action"),
            ("riskLevel", "Risk"),
        ] {
            if let Some(value) = metadata[key]
                .as_str()
                .filter(|value| !value.trim().is_empty())
            {
                prompt.push_str(&format!("\n{label}: {value}"));
            }
        }
        for parameter in metadata["tool_params_display"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let (Some(label), Some(value)) = (
                parameter["display_name"]
                    .as_str()
                    .or_else(|| parameter["name"].as_str()),
                parameter["value"].as_str(),
            ) {
                prompt.push_str(&format!("\n{label}: {value}"));
            }
        }
        Ok(Self {
            prompt,
            request: request.clone(),
        })
    }

    fn supports(&self, persistence: &str) -> bool {
        self.request["_meta"]["persist"]
            .as_array()
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| value.as_str() == Some(persistence))
            })
    }

    pub fn options(&self) -> Vec<String> {
        let mut options = vec![
            "Decline".into(),
            if self.supports("session") {
                "Allow for this session".into()
            } else {
                "Allow".into()
            },
        ];
        if self.supports("always") {
            options.push("Always allow".into());
        }
        options
    }

    pub fn reply(&self, choice: Option<&str>, cancel: bool) -> Value {
        let accepted = !cancel
            && choice.is_some_and(|choice| {
                choice != "Decline" && self.options().iter().any(|option| option == choice)
            });
        let persistence = match choice {
            Some("Always allow") if self.supports("always") => Some("always"),
            Some("Allow for this session") if self.supports("session") => Some("session"),
            _ => None,
        };
        let mut reply = json!({
            "action": if cancel { "cancel" } else if accepted { "accept" } else { "decline" },
            "content": if accepted && !cancel { json!({}) } else { Value::Null },
        });
        if accepted && let Some(persistence) = persistence {
            reply["_meta"] = json!({"persist": persistence});
        }
        reply
    }
}

#[cfg(test)]
#[path = "mcp_confirmation_tests.rs"]
mod tests;
