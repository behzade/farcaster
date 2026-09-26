use super::*;

impl StateStore {
    pub fn load_project_list(&self) -> Result<crate::projects::ProjectList, String> {
        let mut project_states = Vec::<(PathBuf, bool)>::new();
        let mut project_indexes = BTreeMap::<PathBuf, usize>::new();
        let mut statement = self
            .connection
            .prepare("SELECT path, deleted_at FROM projects ORDER BY added_ms, path")
            .map_err(|error| format!("read projects: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
            })
            .map_err(|error| format!("query projects: {error}"))?;
        for row in rows {
            let (path, deleted_at) = row.map_err(|error| error.to_string())?;
            let Some(path) = existing_directory(&path) else {
                continue;
            };
            if let Some(index) = project_indexes.get(&path) {
                // A visible row takes precedence over a legacy hidden alias. Registry callers
                // cannot otherwise restore a project that the same persisted state also hides.
                project_states[*index].1 &= deleted_at.is_some();
            } else {
                project_indexes.insert(path.clone(), project_states.len());
                project_states.push((path, deleted_at.is_some()));
            }
        }
        let (projects, excluded_projects) = project_states.into_iter().fold(
            (Vec::new(), Vec::new()),
            |(mut projects, mut excluded_projects), (path, excluded)| {
                if excluded {
                    excluded_projects.push(path);
                } else {
                    projects.push(path);
                }
                (projects, excluded_projects)
            },
        );
        Ok(crate::projects::ProjectList {
            projects,
            excluded_projects,
        })
    }

    pub fn load_registry(&self) -> Result<Registry, String> {
        let projects = self.load_project_list()?;
        Ok(Registry {
            projects: projects.projects,
            excluded_projects: projects.excluded_projects,
            drafts: self.load_drafts()?,
        })
    }

    pub fn save_project_list(
        &mut self,
        projects: &crate::projects::ProjectList,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start project update: {error}"))?;
        save_projects(
            &transaction,
            &projects.projects,
            &projects.excluded_projects,
        )?;
        transaction
            .commit()
            .map_err(|error| format!("commit project update: {error}"))
    }

    pub fn save_registry(&mut self, registry: &Registry) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start registry update: {error}"))?;
        save_projects(
            &transaction,
            &registry.projects,
            &registry.excluded_projects,
        )?;
        let kept = registry
            .drafts
            .iter()
            .map(|draft| draft.id.as_str())
            .collect::<HashSet<_>>();
        let stale = transaction
            .prepare("SELECT client_key FROM sessions WHERE client_key IS NOT NULL")
            .map_err(|error| format!("read draft keys: {error}"))?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query draft keys: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("decode draft keys: {error}"))?;
        for key in stale {
            if !kept.contains(key.as_str()) {
                remove_draft_row(&transaction, &key)?;
            }
        }
        for draft in &registry.drafts {
            save_draft(&transaction, draft)?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit registry update: {error}"))
    }
}

pub(super) fn save_projects(
    transaction: &Transaction<'_>,
    projects: &[PathBuf],
    excluded_projects: &[PathBuf],
) -> Result<(), String> {
    let now = u64_to_i64(now_ms());
    let projects = unique_project_paths(projects);
    let active_projects = projects.iter().cloned().collect::<HashSet<_>>();
    let excluded_projects = unique_project_paths(excluded_projects)
        .into_iter()
        .filter(|project| !active_projects.contains(project))
        .collect::<Vec<_>>();
    for (index, project) in projects.iter().enumerate() {
        let project_id = ensure_project(transaction, project, now.saturating_add(index as i64))?;
        transaction
            .execute(
                "UPDATE projects SET deleted_at=NULL WHERE id=?1",
                [project_id],
            )
            .map_err(|error| format!("restore registered project: {error}"))?;
    }
    for project in &excluded_projects {
        let project_id = ensure_project(transaction, project, now)?;
        transaction
            .execute(
                "UPDATE projects SET deleted_at=?2 WHERE id=?1",
                params![project_id, now],
            )
            .map_err(|error| format!("exclude project {}: {error}", project.display()))?;
    }
    Ok(())
}

fn existing_directory(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path).canonicalize().ok()?;
    path.is_dir().then_some(path)
}

fn unique_project_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .iter()
        .map(|path| crate::sessions::normalize_session_path(path))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}
