use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, Write},
    path::Path,
};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{connection::CodexConnection, contract::CodexUserInput};

#[derive(Default)]
pub(super) struct Skills(Vec<Skill>);

#[derive(Deserialize)]
struct Skill {
    name: String,
    path: String,
    description: String,
    enabled: bool,
}

#[derive(Deserialize)]
struct SkillList {
    data: Vec<SkillDirectory>,
}

#[derive(Deserialize)]
struct SkillDirectory {
    cwd: String,
    skills: Vec<Skill>,
}

impl Skills {
    pub(super) fn load<R: BufRead, W: Write>(
        connection: &mut CodexConnection<R, W>,
        project: &Path,
    ) -> Self {
        let result = (|| {
            let id = connection.send_request("skills/list", Self::params(project, false))?;
            Self::parse(connection.wait_response(&id)?, project)
        })();
        result.unwrap_or_else(|error| {
            zlog::warn!("Codex skills could not be loaded: {error}");
            Self::default()
        })
    }

    pub(super) fn params(project: &Path, reload: bool) -> Value {
        json!({"cwds": [project], "forceReload": reload})
    }

    pub(super) fn parse(response: Value, project: &Path) -> Result<Self, String> {
        let list: SkillList = serde_json::from_value(response)
            .map_err(|error| format!("decode Codex skills: {error}"))?;
        let mut skills: Vec<Skill> = list
            .data
            .into_iter()
            .filter(|entry| Path::new(&entry.cwd) == project)
            .flat_map(|entry| entry.skills)
            .filter(|skill| {
                skill.enabled && !skill.name.is_empty() && Path::new(&skill.path).is_absolute()
            })
            .collect();
        // Keep one entry per name, but omit names that refer to different paths.
        let mut paths = HashMap::new();
        let mut ambiguous = HashSet::new();
        skills.retain(|skill| match paths.get(&skill.name) {
            None => {
                paths.insert(skill.name.clone(), skill.path.clone());
                true
            }
            Some(path) => {
                if path != &skill.path {
                    ambiguous.insert(skill.name.clone());
                }
                false
            }
        });
        skills.retain(|skill| !ambiguous.contains(&skill.name));
        Ok(Self(skills))
    }

    pub(super) fn commands(&self) -> Vec<Value> {
        self.0
            .iter()
            .map(|skill| {
                json!({
                    "name": format!("skill:{}", skill.name),
                    "description": skill.description,
                    "source": "skill",
                })
            })
            .collect()
    }

    pub(super) fn input(&self, message: String) -> Vec<CodexUserInput> {
        let mut text = String::with_capacity(message.len());
        let mut selected: Vec<&Skill> = Vec::new();
        for part in message.split_inclusive(char::is_whitespace) {
            let token = part
                .trim_end_matches(char::is_whitespace)
                .trim_end_matches(['.', ',', ';', ':', '!', '?']);
            let name = token
                .strip_prefix('$')
                .map(|name| name.strip_prefix("skill:").unwrap_or(name))
                .or_else(|| {
                    if text.is_empty() {
                        token.strip_prefix("/skill:")
                    } else {
                        None
                    }
                });
            if let Some(skill) =
                name.and_then(|name| self.0.iter().find(|skill| skill.name == name))
            {
                text.push('$');
                text.push_str(&skill.name);
                text.push_str(&part[token.len()..]);
                if !selected.iter().any(|other| other.path == skill.path) {
                    selected.push(skill);
                }
            } else {
                text.push_str(part);
            }
        }
        let mut input = vec![CodexUserInput::text(text)];
        for skill in selected {
            input.push(CodexUserInput::Skill {
                name: skill.name.clone(),
                path: skill.path.clone(),
            });
        }
        input
    }
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
