use crate::{Question, QuestionOption, QuestionRequest, QuestionResponse};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) struct Form {
    pub request: QuestionRequest,
    fields: BTreeMap<String, Field>,
}

struct Field {
    schema: Value,
    required: bool,
    choices: BTreeMap<String, Value>,
    items: Vec<(String, String)>,
}

impl Form {
    pub fn parse(id: String, params: &Value) -> Option<Self> {
        if params["mode"].as_str().unwrap_or("form") != "form" {
            return None;
        }
        let message = params["message"].as_str()?;
        let schema = params
            .get("requestedSchema")
            .or_else(|| params.get("requested_schema"))?;
        let properties = schema["properties"].as_object()?;
        if schema["type"] != "object" || properties.is_empty() {
            return None;
        }
        let mut fields = BTreeMap::new();
        let mut questions = Vec::new();
        let mut item_prefix = "item:".to_owned();
        while properties.keys().any(|key| key.starts_with(&item_prefix)) {
            item_prefix.push(':');
        }
        for (key, spec) in properties {
            let kind = spec["type"].as_str()?;
            if !matches!(kind, "string" | "boolean" | "number" | "integer" | "array") {
                return None;
            }
            let mut choices = BTreeMap::new();
            let mut options = Vec::new();
            let mut add = |label: &str, value: Value| -> Option<()> {
                if choices.insert(label.into(), value).is_some() {
                    return None;
                }
                options.push(QuestionOption {
                    label: label.into(),
                    description: String::new(),
                });
                Some(())
            };
            if kind == "boolean" {
                add("Yes", true.into())?;
                add("No", false.into())?;
            } else if let Some(values) = spec["oneOf"].as_array() {
                for v in values {
                    add(
                        v["title"].as_str().or(v["const"].as_str())?,
                        v.get("const")?.clone(),
                    )?;
                }
            } else if let Some(values) = spec["enum"].as_array() {
                for (index, v) in values.iter().enumerate() {
                    add(
                        spec["enumNames"]
                            .get(index)
                            .and_then(Value::as_str)
                            .unwrap_or(v.as_str()?),
                        v.clone(),
                    )?;
                }
            }
            let required = schema["required"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|k| k == key));
            let title = spec["title"].as_str().unwrap_or(key);
            let prompt = format!(
                "{message}\n{}",
                spec["description"].as_str().unwrap_or(title)
            );
            if kind == "array" {
                let item = &spec["items"];
                let values = item["enum"]
                    .as_array()
                    .or_else(|| item["anyOf"].as_array())?;
                let mut items = Vec::with_capacity(values.len());
                for (index, value) in values.iter().enumerate() {
                    let native = value.as_str().or_else(|| value["const"].as_str())?;
                    let label = value["title"].as_str().unwrap_or(native);
                    let id = format!("{item_prefix}{}:{key}:{index}", key.len());
                    questions.push(Question {
                        id: id.clone(),
                        header: title.into(),
                        prompt: format!("{prompt}\nInclude {label}?"),
                        options: ["Yes", "No"]
                            .map(|answer| QuestionOption {
                                label: answer.into(),
                                description: String::new(),
                            })
                            .into(),
                        allow_free_text: false,
                        secret: false,
                    });
                    items.push((id, native.to_owned()));
                }
                if items.is_empty() {
                    return None;
                }
                fields.insert(
                    key.clone(),
                    Field {
                        schema: spec.clone(),
                        required,
                        choices,
                        items,
                    },
                );
                continue;
            }
            if !required {
                let mut skip = "Skip".to_owned();
                while choices.contains_key(&skip) {
                    skip.push(' ');
                }
                options.push(QuestionOption {
                    label: skip.clone(),
                    description: "Leave this field unset".into(),
                });
                choices.insert(skip, Value::Null);
            }
            let allow_free_text =
                kind != "boolean" && !spec["enum"].is_array() && !spec["oneOf"].is_array();
            questions.push(Question {
                id: key.clone(),
                header: title.into(),
                prompt,
                options,
                allow_free_text,
                secret: spec["format"] == "password",
            });
            fields.insert(
                key.clone(),
                Field {
                    schema: spec.clone(),
                    required,
                    choices,
                    items: Vec::new(),
                },
            );
        }
        Some(Self {
            request: QuestionRequest {
                id,
                blocking: true,
                questions,
            },
            fields,
        })
    }

    pub fn reply(&self, response: &QuestionResponse) -> Result<Value, String> {
        if response.cancelled {
            return Ok(json!({"action":"cancel"}));
        }
        let mut content = serde_json::Map::new();
        for (key, field) in &self.fields {
            if !field.items.is_empty() {
                let mut selected = Vec::new();
                for (id, value) in &field.items {
                    match response
                        .answers
                        .get(id)
                        .and_then(|a| a.first())
                        .map(String::as_str)
                    {
                        Some("Yes") => selected.push(value.clone()),
                        Some("No") => {}
                        _ => return Err(format!("Missing selection for {key}")),
                    }
                }
                if selected.is_empty() && !field.required {
                    continue;
                }
                let value = json!(selected);
                if !field.accepts(&value) {
                    return Err(format!("Invalid number of selections for {key}"));
                }
                content.insert(key.clone(), value);
                continue;
            }
            let Some(answer) = response.answers.get(key).and_then(|a| a.first()) else {
                if field.required {
                    return Err(format!("Missing answer for {key}"));
                }
                continue;
            };
            let kind = field.schema["type"].as_str().expect("validated form field");
            let value = if let Some(v) = field.choices.get(answer) {
                v.clone()
            } else if !self
                .request
                .questions
                .iter()
                .any(|q| q.id == *key && q.allow_free_text)
            {
                return Err(format!("Unknown option for {key}"));
            } else if kind == "string" {
                Value::String(answer.clone())
            } else {
                let v: Value = serde_json::from_str(answer)
                    .map_err(|_| format!("Expected {} for {key}", kind))?;
                if !match kind {
                    "integer" => v.is_i64() || v.is_u64(),
                    "number" => v.is_number(),
                    _ => false,
                } {
                    return Err(format!("Expected {} for {key}", kind));
                }
                v
            };
            if !value.is_null() {
                if !field.accepts(&value) {
                    return Err(format!("Answer for {key} is outside the allowed values"));
                }
                content.insert(key.clone(), value);
            }
        }
        Ok(json!({"action":"accept","content":content}))
    }
}

impl Field {
    fn accepts(&self, value: &Value) -> bool {
        let s = &self.schema;
        if let Some(items) = value.as_array() {
            let item = &s["items"];
            let allowed = |v: &Value| {
                v.is_string()
                    && item["enum"]
                        .as_array()
                        .is_none_or(|choices| choices.contains(v))
                    && item["anyOf"]
                        .as_array()
                        .is_none_or(|choices| choices.iter().any(|choice| choice["const"] == *v))
            };
            return items.iter().all(allowed)
                && s["minItems"]
                    .as_u64()
                    .is_none_or(|min| items.len() as u64 >= min)
                && s["maxItems"]
                    .as_u64()
                    .is_none_or(|max| items.len() as u64 <= max);
        }
        if let Some(n) = value.as_f64() {
            return s["minimum"].as_f64().is_none_or(|min| n >= min)
                && s["maximum"].as_f64().is_none_or(|max| n <= max);
        }
        if let Some(text) = value.as_str() {
            let len = text.chars().count() as u64;
            return s["minLength"].as_u64().is_none_or(|min| len >= min)
                && s["maxLength"].as_u64().is_none_or(|max| len <= max);
        }
        true
    }
}
