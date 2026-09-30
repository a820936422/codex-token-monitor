use super::model::{CallRecord, Catalog, Conversation, Project};
use super::store::Store;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

pub(super) fn build(
    store: &Store,
    titles: &HashMap<String, (i64, String)>,
    roots: &HashMap<String, String>,
    cache: &mut HashMap<PathBuf, PathBuf>,
) -> Catalog {
    let mut chosen: BTreeMap<&str, &CallRecord> = BTreeMap::new();
    for call in store.calls.values().rev() {
        chosen.entry(&call.conversation_id).or_insert(call);
    }
    let mut used_paths = HashSet::new();
    let mut conversations = Vec::new();
    for (id, call) in chosen {
        let cwd = roots.get(id).cloned().or_else(|| call.cwd.clone());
        let project_path = cwd
            .as_ref()
            .filter(|cwd| PathBuf::from(cwd).is_absolute())
            .map(|cwd| {
                let path = PathBuf::from(cwd);
                used_paths.insert(path.clone());
                cache
                    .entry(path.clone())
                    .or_insert_with(|| {
                        path.ancestors()
                            .take(128)
                            .find(|ancestor| ancestor.join(".git").exists())
                            .map(PathBuf::from)
                            .unwrap_or(path)
                    })
                    .clone()
            });
        let project_id = project_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "__unassigned__".into());
        let project_name = project_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Other / Unassigned".into());
        conversations.push(Conversation {
            id: id.to_owned(),
            title: titles
                .get(id)
                .map(|(_, title)| title.clone())
                .unwrap_or_else(|| "Untitled conversation".into()),
            cwd,
            project_id,
            project_name,
            project_path: project_path.map(|p| p.to_string_lossy().into_owned()),
        });
    }
    cache.retain(|path, _| used_paths.contains(path));
    conversations.sort_by_cached_key(|c| (c.title.to_lowercase(), c.id.clone()));
    let mut projects: HashMap<String, Project> = HashMap::new();
    for c in &conversations {
        let project = projects
            .entry(c.project_id.clone())
            .or_insert_with(|| Project {
                id: c.project_id.clone(),
                name: c.project_name.clone(),
                path: c.project_path.clone(),
                conversations: 0,
            });
        project.conversations += 1;
    }
    let mut projects: Vec<_> = projects.into_values().collect();
    projects.sort_by_cached_key(|p| (p.name.to_lowercase(), p.id.clone()));
    Catalog {
        projects,
        conversations,
    }
}
