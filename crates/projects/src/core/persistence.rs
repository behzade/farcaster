use super::super::contract::ProjectList;

pub trait ProjectStore {
    fn load_projects(&self) -> Result<ProjectList, String>;
    fn save_projects(&mut self, projects: &ProjectList) -> Result<(), String>;
}

pub fn load_projects(store: &impl ProjectStore) -> Result<ProjectList, String> {
    store.load_projects()
}

pub fn save_projects(store: &mut impl ProjectStore, projects: &ProjectList) -> Result<(), String> {
    store.save_projects(projects)
}
