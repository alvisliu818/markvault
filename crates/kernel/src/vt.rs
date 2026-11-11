use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;

/// Virtual document tree persisted state.
/// Ported semantics from obsidian-virtual-tree (data.json) and
/// siyuan-virtual-tree (virtual-tree.json).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VtStore {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Designated root doc paths of relation trees
    #[serde(default)]
    pub roots: Vec<String>,
    /// Drag order: parent key (doc path, "__root__" for top) -> ordered doc paths
    #[serde(default)]
    pub custom_order: BTreeMap<String, Vec<String>>,
    /// User-curated mount tree
    #[serde(default)]
    pub mount_roots: Vec<MountItem>,
    #[serde(default)]
    pub settings: VtSettings,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VtSettings {
    /// "name" | "weight" fallback sort (drag order always first)
    #[serde(default = "default_sort")]
    pub sort_method: String,
    #[serde(default)]
    pub case_sensitive: bool,
    /// -1 all expanded, 0 all collapsed, N first N levels
    #[serde(default = "default_expand")]
    pub default_expand_level: i32,
    /// Rename delete obsolete ids from custom_order
    #[serde(default = "default_true")]
    pub prune_order: bool,
}

impl Default for VtSettings {
    fn default() -> Self {
        VtSettings {
            sort_method: default_sort(),
            case_sensitive: false,
            default_expand_level: default_expand(),
            prune_order: true,
        }
    }
}
fn default_sort() -> String {
    "name".into()
}
fn default_expand() -> i32 {
    -1
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MountItem {
    pub uid: String,
    /// "file" | "doc" | "notebook"
    pub kind: String,
    /// display name snapshot (rendering resolves live name)
    pub name: String,
    /// vault-relative path (file/doc) or notebook name
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub children: Vec<MountItem>,
}

impl VtStore {
    pub fn load(state: &AppState) -> VtStore {
        let path = state.storage_dir().join("vt.json");
        match fs::read_to_string(&path) {
            Ok(s) if !s.trim().is_empty() => serde_json::from_str(&s).unwrap_or_default(),
            _ => VtStore::default(),
        }
    }

    pub fn save(&self, state: &AppState) {
        let dir = state.storage_dir();
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vt.json");
        let tmp = dir.join("vt.json.tmp");
        if fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default()).is_ok() {
            let _ = fs::rename(tmp, path);
        }
    }
}

/// Relation-tree node returned to the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VtNode {
    pub id: String,
    pub path: String,
    pub name: String,
    pub weight: f64,
    pub is_root: bool,
    pub children: Vec<VtNode>,
}

/// A doc's vt metadata pulled from the index
#[derive(Debug, Clone)]
pub struct DocInfo {
    pub path: String,
    pub title: String,
    pub parents: Vec<String>,
    pub weight: f64,
}

pub fn load_doc_infos(state: &AppState) -> Vec<DocInfo> {
    let db = state.db.lock().unwrap();
    let (parent_attr, weight_attr) = {
        let conf = state.conf.read().unwrap();
        (conf.parent_attr.clone(), conf.weight_attr.clone())
    };
    let mut out = Vec::new();
    if let Ok(mut stmt) = db.prepare("SELECT path, title, fm_json FROM docs") {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                let (path, title, fm_json) = row;
                let fm: Value = serde_json::from_str(&fm_json).unwrap_or(Value::Null);
                let parents = fm
                    .get(&parent_attr)
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .or_else(|| {
                        // single string also accepted
                        fm.get(&parent_attr)
                            .and_then(|v| v.as_str())
                            .map(|s| vec![s.to_string()])
                    })
                    .unwrap_or_default();
                let weight = fm.get(&weight_attr).and_then(|v| v.as_f64()).unwrap_or(0.0);
                out.push(DocInfo {
                    path,
                    title,
                    parents,
                    weight,
                });
            }
        }
    }
    out
}

/// Resolve a wikilink-ish target to a doc path (Obsidian shortest-path style):
/// exact path match (with or without .md) > title match > basename match.
pub fn resolve_target(target: &str, docs: &[DocInfo]) -> Option<String> {
    let t = crate::parse::split_wikilink(target).0;
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    let with_md = if t.ends_with(".md") {
        t.to_string()
    } else {
        format!("{}.md", t)
    };
    if let Some(d) = docs.iter().find(|d| d.path == with_md || d.path == t) {
        return Some(d.path.clone());
    }
    if let Some(d) = docs.iter().find(|d| d.title == t) {
        return Some(d.path.clone());
    }
    // basename match: prefer shortest path for stability
    docs.iter()
        .filter(|d| {
            d.path
                .rsplit('/')
                .next()
                .and_then(|s| s.strip_suffix(".md"))
                .map(|base| base == t)
                .unwrap_or(false)
        })
        .min_by_key(|d| d.path.len())
        .map(|d| d.path.clone())
}

/// Build relation trees (port of obsidian-virtual-tree treeBuilder.buildTree,
/// with siyuan-virtual-tree three-tier sorting).
pub fn build_relation_tree(
    docs: &[DocInfo],
    roots: &[String],
    settings: &VtSettings,
    custom_order: &BTreeMap<String, Vec<String>>,
) -> Vec<VtNode> {
    let mut by_path: HashMap<&str, &DocInfo> = HashMap::new();
    let mut children: HashMap<String, Vec<&DocInfo>> = HashMap::new();

    for d in docs {
        by_path.insert(d.path.as_str(), d);
        for p in &d.parents {
            if let Some(parent_path) = resolve_target(p, docs) {
                if parent_path != d.path {
                    children.entry(parent_path).or_default().push(d);
                }
            }
        }
    }

    // Roots: explicit; fallback to childful parentless docs; loose fallback for
    // pure-cycle groups (childful docs even if they have parents)
    let root_paths: Vec<String> = if roots.is_empty() {
        let strict: Vec<String> = docs
            .iter()
            .filter(|d| children.contains_key(d.path.as_str()))
            .filter(|d| {
                !d.parents
                    .iter()
                    .any(|p| resolve_target(p, docs).is_some())
            })
            .map(|d| d.path.clone())
            .collect();
        if strict.is_empty() {
            docs.iter()
                .filter(|d| children.contains_key(d.path.as_str()))
                .map(|d| d.path.clone())
                .collect()
        } else {
            strict
        }
    } else {
        roots.to_vec()
    };

    let mut out = Vec::new();
    for root in root_paths {
        if let Some(d) = by_path.get(root.as_str()) {
            let mut ancestors = HashSet::new();
            ancestors.insert(d.path.clone());
            let node = build_subtree(
                d,
                &by_path,
                &children,
                &mut ancestors,
                settings,
                custom_order,
                vec![d.path.clone()],
                true,
            );
            out.push(node);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn build_subtree(
    doc: &DocInfo,
    by_path: &HashMap<&str, &DocInfo>,
    children: &HashMap<String, Vec<&DocInfo>>,
    ancestors: &mut HashSet<String>,
    settings: &VtSettings,
    custom_order: &BTreeMap<String, Vec<String>>,
    chain: Vec<String>,
    is_root: bool,
) -> VtNode {
    let kid_docs = children
        .get(doc.path.as_str())
        .cloned()
        .unwrap_or_default();

    // sort kids: custom order rank > weight desc > name asc
    let order_key = chain.join("::");
    let rank: HashMap<&String, usize> = custom_order
        .get(&order_key)
        .map(|ids| ids.iter().enumerate().map(|(i, id)| (id, i)).collect())
        .unwrap_or_default();

    let mut kids: Vec<&DocInfo> = kid_docs;
    kids.sort_by(|a, b| {
        let ra = rank.get(&a.path);
        let rb = rank.get(&b.path);
        match (ra, rb) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => {
                // weight desc
                b.weight
                    .partial_cmp(&a.weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        if settings.case_sensitive {
                            a.title.cmp(&b.title)
                        } else {
                            a.title
                                .to_lowercase()
                                .cmp(&b.title.to_lowercase())
                                .then_with(|| a.title.cmp(&b.title))
                        }
                    })
            }
        }
    });

    // disambiguate duplicate display names among siblings
    let names = disambiguate_names(&kids);

    let mut children_nodes = Vec::new();
    for kid in kids {
        if ancestors.contains(&kid.path) {
            continue; // cycle skip (per-branch)
        }
        ancestors.insert(kid.path.clone());
        let mut kid_chain = chain.clone();
        kid_chain.push(kid.path.clone());
        let mut node = build_subtree(
            kid,
            by_path,
            children,
            ancestors,
            settings,
            custom_order,
            kid_chain,
            false,
        );
        // display name comes from THIS level's disambiguation map
        node.name = names
            .get(&kid.path)
            .cloned()
            .unwrap_or_else(|| kid.title.clone());
        ancestors.remove(&kid.path);
        children_nodes.push(node);
    }

    VtNode {
        id: chain.join("::"),
        path: doc.path.clone(),
        name: names.get(&doc.path).cloned().unwrap_or_else(|| doc.title.clone()),
        weight: doc.weight,
        is_root,
        children: children_nodes,
    }
}

/// Port of obsidian-virtual-tree treeBuilder.ts:129-214 — when siblings share
/// a display name, append reversed folder segments joined with `<` until unique.
fn disambiguate_names(kids: &[&DocInfo]) -> HashMap<String, String> {
    let mut result: HashMap<String, String> = HashMap::new();
    let mut groups: HashMap<&str, Vec<&&DocInfo>> = HashMap::new();
    for k in kids {
        groups.entry(k.title.as_str()).or_default().push(k);
    }
    for (name, group) in groups {
        if group.len() == 1 {
            result.insert(group[0].path.clone(), name.to_string());
            continue;
        }
        // progressively add reversed folder segments
        let mut depth = 0usize;
        let mut pending: Vec<&&DocInfo> = group.clone();
        while pending.len() > 1 && depth < 16 {
            let mut next_map: HashMap<String, Vec<&&DocInfo>> = HashMap::new();
            for k in &pending {
                let seg = folder_segment(&k.path, depth);
                let label = format!("{}<{}", name, seg);
                next_map.entry(label).or_default().push(k);
            }
            let mut still: Vec<&&DocInfo> = Vec::new();
            for (label, ks) in next_map {
                if ks.len() == 1 {
                    result.insert(ks[0].path.clone(), label);
                } else {
                    still.extend(ks);
                }
            }
            pending = still;
            depth += 1;
        }
        for k in &pending {
            result.insert(k.path.clone(), k.path.clone());
        }
    }
    result
}

/// nth folder segment from the deepest: depth 0 = immediate parent dir name
fn folder_segment(path: &str, depth: usize) -> String {
    let parts: Vec<&str> = path.rsplit('/').skip(1).collect(); // skip filename
    if depth < parts.len() {
        parts[depth].to_string()
    } else {
        "(root)".into()
    }
}

/// Keep custom_order in sync after a doc rename/move: rewrite path prefixes.
pub fn sync_rename(store: &mut VtStore, old_path: &str, new_path: &str) {
    let rewrite = |s: &mut String| {
        if s == old_path {
            *s = new_path.to_string();
        } else if s.starts_with(&format!("{}/", old_path)) {
            *s = format!("{}{}", new_path, &s[old_path.len()..]);
        }
    };
    for r in &mut store.roots {
        rewrite(r);
    }
    for list in store.custom_order.values_mut() {
        for p in list.iter_mut() {
            rewrite(p);
        }
    }
    for k in store.custom_order.keys().cloned().collect::<Vec<_>>() {
        let mut key = k.clone();
        rewrite(&mut key);
        if key != k {
            if let Some(v) = store.custom_order.remove(&k) {
                store.custom_order.insert(key, v);
            }
        }
    }
    sync_mount_rename(&mut store.mount_roots, old_path, new_path);
}

fn sync_mount_rename(items: &mut Vec<MountItem>, old_path: &str, new_path: &str) {
    for it in items.iter_mut() {
        if it.path == old_path {
            it.path = new_path.to_string();
            if it.kind == "file" || it.kind == "doc" {
                if let Some(name) = new_path.rsplit('/').next() {
                    it.name = name.trim_end_matches(".md").to_string();
                }
            }
        } else if it.path.starts_with(&format!("{}/", old_path)) {
            it.path = format!("{}{}", new_path, &it.path[old_path.len()..]);
        }
        sync_mount_rename(&mut it.children, old_path, new_path);
    }
}

/// Remove deleted docs from tree state
pub fn sync_delete(store: &mut VtStore, path: &str) {
    store.roots.retain(|r| r != path && !r.starts_with(&format!("{}/", path)));
    for list in store.custom_order.values_mut() {
        list.retain(|p| p != path && !p.starts_with(&format!("{}/", path)));
    }
    let is_dir = !path.contains('.');
    sync_mount_delete(&mut store.mount_roots, path, is_dir);
}

fn sync_mount_delete(items: &mut Vec<MountItem>, path: &str, is_dir: bool) {
    items.retain(|it| {
        if is_dir {
            !(it.path == path || it.path.starts_with(&format!("{}/", path)))
        } else {
            it.path != path
        }
    });
    for it in items.iter_mut() {
        sync_mount_delete(&mut it.children, path, is_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(path: &str, title: &str, parents: Vec<String>, weight: f64) -> DocInfo {
        DocInfo {
            path: path.into(),
            title: title.into(),
            parents,
            weight,
        }
    }

    #[test]
    fn test_build_and_cycle() {
        let docs = vec![
            doc("n/A.md", "A", vec!["[[B]]".into()], 0.0),
            doc("n/B.md", "B", vec!["[[A]]".into()], 0.0), // cycle
            doc("n/C.md", "C", vec!["[[A]]".into()], 1.0),
        ];
        let tree = build_relation_tree(&docs, &[], &VtSettings::default(), &BTreeMap::new());
        // cycle group: loose fallback roots = childful docs (A, B); recursion must terminate
        assert!(!tree.is_empty());
    }

    #[test]
    fn test_sort_weight_desc_name_asc() {
        let docs = vec![
            doc("n/A.md", "A", vec![], 0.0),
            doc("n/x.md", "x", vec!["[[A]]".into()], 0.0),
            doc("n/z.md", "z", vec!["[[A]]".into()], 5.0),
            doc("n/y.md", "y", vec!["[[A]]".into()], 5.0),
        ];
        let tree = build_relation_tree(
            &docs,
            &["n/A.md".to_string()],
            &VtSettings::default(),
            &BTreeMap::new(),
        );
        let kids: Vec<&str> = tree[0].children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(kids, vec!["y", "z", "x"]);
    }

    #[test]
    fn test_custom_order_first() {
        let docs = vec![
            doc("n/A.md", "A", vec![], 0.0),
            doc("n/1.md", "1", vec!["[[A]]".into()], 9.0),
            doc("n/2.md", "2", vec!["[[A]]".into()], 0.0),
        ];
        let mut order = BTreeMap::new();
        order.insert(
            "n/A.md".to_string(),
            vec!["n/2.md".to_string(), "n/1.md".to_string()],
        );
        let tree = build_relation_tree(
            &docs,
            &["n/A.md".to_string()],
            &VtSettings::default(),
            &order,
        );
        let kids: Vec<&str> = tree[0].children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(kids, vec!["2", "1"]);
    }

    #[test]
    fn test_disambiguation() {
        let docs = vec![
            doc("n/A.md", "A", vec![], 0.0),
            doc("notes1/Readme.md", "Readme", vec!["[[A]]".into()], 0.0),
            doc("notes2/Readme.md", "Readme", vec!["[[A]]".into()], 0.0),
        ];
        let tree = build_relation_tree(
            &docs,
            &["n/A.md".to_string()],
            &VtSettings::default(),
            &BTreeMap::new(),
        );
        println!("TREE: {}", serde_json::to_string_pretty(&tree).unwrap());
        let kids: Vec<&str> = tree[0].children.iter().map(|c| c.name.as_str()).collect();
        assert!(kids.contains(&"Readme<notes1"));
        assert!(kids.contains(&"Readme<notes2"));
    }

    #[test]
    fn test_resolve_target() {
        let docs = vec![doc("n/My Note.md", "随笔", vec![], 0.0)];
        assert_eq!(resolve_target("[[My Note]]", &docs).as_deref(), Some("n/My Note.md"));
        assert_eq!(resolve_target("[[随笔]]", &docs).as_deref(), Some("n/My Note.md"));
        assert_eq!(resolve_target("n/My Note.md", &docs).as_deref(), Some("n/My Note.md"));
    }
}

