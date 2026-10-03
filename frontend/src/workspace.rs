//! The `Workspace` abstraction: one variant per mode. Remote mode talks to
//! the backend file API; local mode uses the File System Access API directly.
//! All paths are project-relative (the project root is "").

use leptos::prelude::WithValue;
use openwebide_core::{FileEntry, SearchHit, vfs::SearchOptions};
use web_sys::FileSystemDirectoryHandle;

use crate::backend::Api;
use crate::local_fs;

pub enum Workspace {
    Remote { api: Api, project_id: i64 },
    Local { handle: FileSystemDirectoryHandle },
}

impl Workspace {
    pub async fn list(&self, dir: &str) -> Result<Vec<FileEntry>, String> {
        let mut entries = match self {
            Workspace::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .list_files(*project_id, dir)
                    .await
            }
            Workspace::Local { handle } => local_fs::list(handle, dir).await,
        }?;
        openwebide_core::vfs::sort_file_entries(&mut entries);
        Ok(entries)
    }

    pub async fn read(&self, path: &str) -> Result<String, String> {
        match self {
            Workspace::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .read_file(*project_id, path)
                    .await
            }
            Workspace::Local { handle } => local_fs::read(handle, path).await,
        }
    }

    pub async fn read_lossy(&self, path: &str) -> Result<String, String> {
        match self {
            Workspace::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .read_file_lossy(*project_id, path)
                    .await
            }
            Workspace::Local { handle } => local_fs::read_lossy(handle, path).await,
        }
    }

    pub async fn read_blob_url(&self, path: &str) -> Result<String, String> {
        match self {
            Workspace::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .read_file_object_url(*project_id, path)
                    .await
            }
            Workspace::Local { handle } => local_fs::read_blob_url(handle, path).await,
        }
    }

    pub async fn write(&self, path: &str, content: &str) -> Result<(), String> {
        match self {
            Workspace::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .write_file(*project_id, path, content)
                    .await
            }
            Workspace::Local { handle } => local_fs::write(handle, path, content).await,
        }
    }

    pub async fn copy(&self, from: &str, to: &str) -> Result<(), String> {
        match self {
            Workspace::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .copy_file(*project_id, from, to)
                    .await
            }
            Workspace::Local { handle } => {
                use openwebide_core::Vfs;
                let vfs = local_fs::BrowserFsaVfs::new(handle.clone());
                vfs.copy(from, to).await.map_err(local_fs::display_error)
            }
        }
    }

    pub async fn create(&self, path: &str, is_dir: bool) -> Result<(), String> {
        match self {
            Workspace::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .create_file(*project_id, path, is_dir)
                    .await
            }
            Workspace::Local { handle } => local_fs::create(handle, path, is_dir).await,
        }
    }

    pub async fn search_content(
        &self,
        query: &str,
        dir: &str,
        opts: SearchOptions,
    ) -> Result<Vec<SearchHit>, String> {
        match self {
            Workspace::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .search_content(*project_id, query, dir, opts)
                    .await
            }
            Workspace::Local { handle } => local_fs::search_content(handle, query, dir, opts).await,
        }
    }

    pub async fn delete(&self, path: &str) -> Result<(), String> {
        match self {
            Workspace::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .delete_file(*project_id, path)
                    .await
            }
            Workspace::Local { handle } => local_fs::delete(handle, path).await,
        }
    }
}
