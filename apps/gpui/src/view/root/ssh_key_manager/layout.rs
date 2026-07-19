use crate::services::ssh_keys::{SshKeyLayout, SshKeyMetadata};

use super::{SshKeyDragItem, SshKeyDropPosition};

pub(super) fn root_items(layout: &SshKeyLayout, keys: &[SshKeyMetadata]) -> Vec<String> {
    let folder_ids = layout
        .folders
        .iter()
        .map(|folder| folder.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut items = layout
        .folders
        .iter()
        .enumerate()
        .map(|(index, folder)| {
            (
                folder.id.clone(),
                layout
                    .item_order
                    .get(&folder.id)
                    .copied()
                    .unwrap_or(((index + 1) * 1000) as u64),
            )
        })
        .chain(keys.iter().filter_map(|key| {
            let assigned = layout
                .assignments
                .get(&key.id)
                .is_some_and(|folder_id| folder_ids.contains(folder_id.as_str()));
            (!assigned).then(|| {
                (
                    key.id.clone(),
                    layout
                        .item_order
                        .get(&key.id)
                        .copied()
                        .unwrap_or(key.imported_at),
                )
            })
        }))
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    items.into_iter().map(|(id, _)| id).collect()
}

fn folder_key_items(
    layout: &SshKeyLayout,
    keys: &[SshKeyMetadata],
    folder_id: &str,
) -> Vec<String> {
    let mut items = keys
        .iter()
        .filter(|key| layout.assignments.get(&key.id).map(String::as_str) == Some(folder_id))
        .map(|key| {
            (
                key.id.clone(),
                layout
                    .item_order
                    .get(&key.id)
                    .copied()
                    .unwrap_or(key.imported_at),
            )
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    items.into_iter().map(|(id, _)| id).collect()
}

fn write_order(layout: &mut SshKeyLayout, items: impl IntoIterator<Item = String>) {
    for (index, id) in items.into_iter().enumerate() {
        layout.item_order.insert(id, ((index + 1) * 1000) as u64);
    }
}

pub(super) fn next_root_order(layout: &SshKeyLayout) -> u64 {
    layout
        .folders
        .iter()
        .filter_map(|folder| layout.item_order.get(&folder.id))
        .chain(
            layout
                .item_order
                .iter()
                .filter(|(id, _)| !layout.assignments.contains_key(*id))
                .map(|(_, order)| order),
        )
        .copied()
        .max()
        .unwrap_or(0)
        .saturating_add(1000)
}

pub(super) fn assign_key_folder(
    layout: &mut SshKeyLayout,
    key_id: &str,
    folder_id: Option<String>,
) {
    let current = layout.assignments.get(key_id).cloned();
    let valid_folder =
        folder_id.filter(|candidate| layout.folders.iter().any(|folder| folder.id == *candidate));
    if current == valid_folder {
        if !layout.item_order.contains_key(key_id) {
            let next_order = next_root_order(layout);
            layout.item_order.insert(key_id.to_string(), next_order);
        }
        return;
    }

    match valid_folder.as_ref() {
        Some(folder_id) => {
            layout
                .assignments
                .insert(key_id.to_string(), folder_id.clone());
        }
        None => {
            layout.assignments.remove(key_id);
        }
    }
    let next_order = match valid_folder {
        Some(folder_id) => layout
            .assignments
            .iter()
            .filter(|(other_key, assigned)| {
                other_key.as_str() != key_id && assigned.as_str() == folder_id
            })
            .filter_map(|(other_key, _)| layout.item_order.get(other_key))
            .copied()
            .max()
            .unwrap_or(0)
            .saturating_add(1000),
        None => next_root_order(layout),
    };
    layout.item_order.insert(key_id.to_string(), next_order);
}

pub(super) fn reorder_relative(
    layout: &mut SshKeyLayout,
    keys: &[SshKeyMetadata],
    dragged: &SshKeyDragItem,
    target: &SshKeyDragItem,
    position: SshKeyDropPosition,
) {
    if dragged == target {
        return;
    }
    let insertion_offset = usize::from(position == SshKeyDropPosition::After);

    match (dragged, target) {
        (SshKeyDragItem::Key(key_id), SshKeyDragItem::Folder(folder_id)) => {
            layout.assignments.remove(key_id);
            let mut siblings = root_items(layout, keys);
            siblings.retain(|id| id != key_id);
            if let Some(target_index) = siblings.iter().position(|id| id == folder_id) {
                siblings.insert(target_index + insertion_offset, key_id.clone());
                write_order(layout, siblings);
            }
        }
        (SshKeyDragItem::Key(key_id), SshKeyDragItem::Key(target_id)) => {
            let folder_ids = layout
                .folders
                .iter()
                .map(|folder| folder.id.as_str())
                .collect::<std::collections::HashSet<_>>();
            let parent = layout
                .assignments
                .get(target_id)
                .filter(|folder_id| folder_ids.contains(folder_id.as_str()))
                .cloned();
            match parent.as_ref() {
                Some(folder_id) => {
                    layout.assignments.insert(key_id.clone(), folder_id.clone());
                }
                None => {
                    layout.assignments.remove(key_id);
                }
            }
            let mut siblings = match parent {
                Some(folder_id) => folder_key_items(layout, keys, &folder_id),
                None => root_items(layout, keys),
            };
            siblings.retain(|id| id != key_id);
            if let Some(target_index) = siblings.iter().position(|id| id == target_id) {
                siblings.insert(target_index + insertion_offset, key_id.clone());
                write_order(layout, siblings);
            }
        }
        (SshKeyDragItem::Folder(folder_id), target) => {
            let mut siblings = root_items(layout, keys);
            let target_id = target.id();
            if !siblings.iter().any(|id| id == target_id) {
                return;
            }
            siblings.retain(|id| id != folder_id);
            if let Some(target_index) = siblings.iter().position(|id| id == target_id) {
                siblings.insert(target_index + insertion_offset, folder_id.clone());
                write_order(layout, siblings);
            }
        }
    }
}

pub(super) fn delete_folder_from_layout(layout: &mut SshKeyLayout, folder_id: &str) {
    layout.folders.retain(|folder| folder.id != folder_id);
    layout
        .assignments
        .retain(|_, assigned| assigned != folder_id);
    layout.item_order.remove(folder_id);
}

pub(super) fn normalize_layout(layout: &mut SshKeyLayout, keys: &[SshKeyMetadata]) {
    let mut folder_ids = std::collections::HashSet::new();
    let mut folder_names = std::collections::HashSet::new();
    layout.folders.retain(|folder| {
        !folder.id.trim().is_empty()
            && !folder.name.trim().is_empty()
            && folder_ids.insert(folder.id.clone())
            && folder_names.insert(folder.name.trim().to_string())
    });
    let key_ids = keys
        .iter()
        .map(|key| key.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    layout.assignments.retain(|key_id, folder_id| {
        key_ids.contains(key_id.as_str()) && folder_ids.contains(folder_id)
    });
    layout
        .item_order
        .retain(|id, _| key_ids.contains(id.as_str()) || folder_ids.contains(id.as_str()));

    write_order(layout, root_items(layout, keys));
    for folder_id in folder_ids {
        write_order(layout, folder_key_items(layout, keys, &folder_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ssh_keys::SshKeyFolder;
    use std::collections::HashMap;

    fn key(id: &str, imported_at: u64) -> SshKeyMetadata {
        SshKeyMetadata {
            id: id.to_string(),
            name: id.to_string(),
            note: Some(format!("note-{id}")),
            algorithm: "ssh-ed25519".to_string(),
            fingerprint: format!("SHA256:{id}"),
            encrypted: false,
            imported_at,
            usage_count: 0,
        }
    }

    fn folder(id: &str, name: &str) -> SshKeyFolder {
        SshKeyFolder {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn layout(folders: Vec<SshKeyFolder>) -> SshKeyLayout {
        SshKeyLayout {
            folders,
            assignments: HashMap::new(),
            item_order: HashMap::new(),
        }
    }

    #[test]
    fn reorders_keys_in_root() {
        let keys = vec![key("key-a", 1000), key("key-b", 2000)];
        let mut layout = layout(Vec::new());

        reorder_relative(
            &mut layout,
            &keys,
            &SshKeyDragItem::Key("key-a".to_string()),
            &SshKeyDragItem::Key("key-b".to_string()),
            SshKeyDropPosition::After,
        );
        assert_eq!(root_items(&layout, &keys), vec!["key-b", "key-a"]);

        reorder_relative(
            &mut layout,
            &keys,
            &SshKeyDragItem::Key("key-a".to_string()),
            &SshKeyDragItem::Key("key-b".to_string()),
            SshKeyDropPosition::Before,
        );
        assert_eq!(root_items(&layout, &keys), vec!["key-a", "key-b"]);
    }

    #[test]
    fn reorders_keys_inside_folder() {
        let keys = vec![key("key-a", 1000), key("key-b", 2000)];
        let mut layout = layout(vec![folder("folder-a", "A")]);
        layout.assignments = HashMap::from([
            ("key-a".to_string(), "folder-a".to_string()),
            ("key-b".to_string(), "folder-a".to_string()),
        ]);

        reorder_relative(
            &mut layout,
            &keys,
            &SshKeyDragItem::Key("key-a".to_string()),
            &SshKeyDragItem::Key("key-b".to_string()),
            SshKeyDropPosition::After,
        );

        assert_eq!(
            folder_key_items(&layout, &keys, "folder-a"),
            vec!["key-b", "key-a"]
        );
    }

    #[test]
    fn moves_key_between_folders() {
        let keys = vec![key("key-a", 1000), key("key-b", 2000)];
        let mut layout = layout(vec![folder("folder-a", "A"), folder("folder-b", "B")]);
        layout.assignments = HashMap::from([
            ("key-a".to_string(), "folder-a".to_string()),
            ("key-b".to_string(), "folder-b".to_string()),
        ]);

        reorder_relative(
            &mut layout,
            &keys,
            &SshKeyDragItem::Key("key-a".to_string()),
            &SshKeyDragItem::Key("key-b".to_string()),
            SshKeyDropPosition::After,
        );

        assert_eq!(
            layout.assignments.get("key-a"),
            Some(&"folder-b".to_string())
        );
        assert_eq!(
            folder_key_items(&layout, &keys, "folder-b"),
            vec!["key-b", "key-a"]
        );
    }

    #[test]
    fn moves_key_back_to_root() {
        let keys = vec![key("key-a", 1000), key("key-b", 2000)];
        let mut layout = layout(vec![folder("folder-a", "A")]);
        layout
            .assignments
            .insert("key-a".to_string(), "folder-a".to_string());

        assign_key_folder(&mut layout, "key-a", None);
        normalize_layout(&mut layout, &keys);

        assert!(!layout.assignments.contains_key("key-a"));
        assert!(root_items(&layout, &keys).contains(&"key-a".to_string()));
    }

    #[test]
    fn reorders_folders_at_root() {
        let keys = Vec::new();
        let mut layout = layout(vec![folder("folder-a", "A"), folder("folder-b", "B")]);

        reorder_relative(
            &mut layout,
            &keys,
            &SshKeyDragItem::Folder("folder-a".to_string()),
            &SshKeyDragItem::Folder("folder-b".to_string()),
            SshKeyDropPosition::After,
        );

        assert_eq!(root_items(&layout, &keys), vec!["folder-b", "folder-a"]);
    }

    #[test]
    fn deleting_folder_unassigns_keys_and_removes_folder_order() {
        let keys = vec![key("key-a", 1000)];
        let mut layout = layout(vec![folder("folder-a", "A")]);
        layout
            .assignments
            .insert("key-a".to_string(), "folder-a".to_string());
        layout.item_order.insert("folder-a".to_string(), 1000);
        layout.item_order.insert("key-a".to_string(), 1000);

        delete_folder_from_layout(&mut layout, "folder-a");
        normalize_layout(&mut layout, &keys);

        assert!(layout.folders.is_empty());
        assert!(layout.assignments.is_empty());
        assert!(!layout.item_order.contains_key("folder-a"));
        assert_eq!(root_items(&layout, &keys), vec!["key-a"]);
    }

    #[test]
    fn assigning_key_to_current_folder_preserves_order() {
        let keys = vec![key("key-a", 1000), key("key-b", 2000)];
        let mut layout = layout(vec![folder("folder-a", "A")]);
        layout.assignments = HashMap::from([
            ("key-a".to_string(), "folder-a".to_string()),
            ("key-b".to_string(), "folder-a".to_string()),
        ]);
        layout.item_order =
            HashMap::from([("key-a".to_string(), 1000), ("key-b".to_string(), 2000)]);

        assign_key_folder(&mut layout, "key-a", Some("folder-a".to_string()));

        assert_eq!(layout.item_order.get("key-a"), Some(&1000));
        assert_eq!(
            folder_key_items(&layout, &keys, "folder-a"),
            vec!["key-a", "key-b"]
        );
    }

    #[test]
    fn normalization_is_idempotent_and_keeps_every_live_id_once() {
        let keys = vec![key("key-a", 1000), key("key-b", 2000)];
        let mut layout = layout(vec![folder("folder-a", "A")]);
        layout
            .assignments
            .insert("key-a".to_string(), "folder-a".to_string());
        layout.item_order = HashMap::from([
            ("folder-a".to_string(), 9000),
            ("key-a".to_string(), 9000),
            ("key-b".to_string(), 9000),
            ("stale".to_string(), u64::MAX),
        ]);

        normalize_layout(&mut layout, &keys);
        let normalized = layout.clone();
        normalize_layout(&mut layout, &keys);

        assert_eq!(layout, normalized);
        assert_eq!(
            layout
                .item_order
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([
                "folder-a".to_string(),
                "key-a".to_string(),
                "key-b".to_string(),
            ])
        );
        assert_eq!(root_items(&layout, &keys), vec!["folder-a", "key-b"]);
        assert_eq!(folder_key_items(&layout, &keys, "folder-a"), vec!["key-a"]);
    }

    #[test]
    fn normalizes_duplicate_and_stale_layout_ids() {
        let keys = vec![key("key-a", 1000)];
        let mut layout = layout(vec![
            folder("folder-a", "A"),
            folder("folder-a", "重复 ID"),
            folder("folder-b", "A"),
            folder("", "无效"),
        ]);
        layout.assignments = HashMap::from([
            ("key-a".to_string(), "folder-a".to_string()),
            ("missing-key".to_string(), "folder-a".to_string()),
        ]);
        layout.item_order = HashMap::from([
            ("key-a".to_string(), 9000),
            ("folder-a".to_string(), 8000),
            ("stale".to_string(), 1),
        ]);

        normalize_layout(&mut layout, &keys);

        assert_eq!(layout.folders, vec![folder("folder-a", "A")]);
        assert_eq!(
            layout.assignments,
            HashMap::from([("key-a".to_string(), "folder-a".to_string())])
        );
        assert_eq!(
            layout
                .item_order
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from(["folder-a".to_string(), "key-a".to_string()])
        );
    }
}
