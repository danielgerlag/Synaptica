use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::engine::{StorageEngine, StorageError, StorageResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMeta {
    pub name: String,
    pub label: String,
    pub created_at: String,
    pub size_bytes: u64,
}

pub struct BackupManager {
    backups_dir: PathBuf,
}

impl BackupManager {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            backups_dir: data_dir.as_ref().join("backups"),
        }
    }

    pub fn create(&self, storage: &StorageEngine, label: &str) -> StorageResult<BackupMeta> {
        let label = if label.is_empty() { "manual" } else { label };
        let now = chrono::Utc::now();
        let backup_name = format!("{}_{}", now.format("%Y%m%d_%H%M%S"), label);
        let backup_dir = self.backups_dir.join(&backup_name);
        std::fs::create_dir_all(&backup_dir)?;
        storage.create_backup(&backup_dir)?;

        let meta = BackupMeta {
            name: backup_name,
            label: label.to_string(),
            created_at: now.to_rfc3339(),
            size_bytes: dir_size(&backup_dir),
        };

        let meta_json = serde_json::to_string_pretty(&serde_json::json!({
            "label": &meta.label,
            "created_at": &meta.created_at,
            "backup_name": &meta.name,
        }))
        .map_err(|e| StorageError::Internal(e.to_string()))?;
        std::fs::write(backup_dir.join("backup_meta.json"), meta_json)?;

        Ok(meta)
    }

    pub fn list(&self) -> StorageResult<Vec<BackupMeta>> {
        let mut backups = Vec::new();
        if !self.backups_dir.exists() {
            return Ok(backups);
        }

        for entry in std::fs::read_dir(&self.backups_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let meta_path = path.join("backup_meta.json");
            if !meta_path.exists() {
                continue;
            }

            let content = std::fs::read_to_string(&meta_path)?;
            let json: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| StorageError::Internal(e.to_string()))?;
            backups.push(BackupMeta {
                name: entry.file_name().to_string_lossy().to_string(),
                label: json["label"].as_str().unwrap_or("").to_string(),
                created_at: json["created_at"].as_str().unwrap_or("").to_string(),
                size_bytes: dir_size(&path),
            });
        }

        backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(backups)
    }

    pub fn delete(&self, name: &str) -> StorageResult<()> {
        let backup_path = self.backups_dir.join(name);
        if !backup_path.exists() {
            return Err(StorageError::NotFound(format!(
                "backup '{}' not found",
                name
            )));
        }

        std::fs::remove_dir_all(&backup_path)?;
        Ok(())
    }
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            } else if path.is_dir() {
                total += dir_size(&path);
            }
        }
    }
    total
}
