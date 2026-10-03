//! Shared workspace facade; adapters supply filesystem/HTTP primitives.
use crate::{backend::Api, local_fs, state::projects::ProjectsState};
use leptos::prelude::{WithUntracked, WithValue};
use openwebide_core::{
    FileEntry, SearchHit, Vfs, VfsError, WorkspaceMode,
    vfs::{SearchOptions, VfsEntryKind, workspace_path},
};
use web_sys::FileSystemDirectoryHandle;

#[derive(Debug)]
pub enum WorkspaceError {
    FileSystem(VfsError),
    Transport(String),
}
impl WorkspaceError {
    pub fn needs_folder_access(&self) -> bool {
        matches!(self, Self::FileSystem(VfsError::PermissionRequired(_)))
    }
}
impl From<VfsError> for WorkspaceError {
    fn from(error: VfsError) -> Self {
        Self::FileSystem(error)
    }
}
impl From<String> for WorkspaceError {
    fn from(error: String) -> Self {
        Self::Transport(error)
    }
}
impl From<WorkspaceError> for String {
    fn from(error: WorkspaceError) -> Self {
        error.to_string()
    }
}
impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileSystem(error) => error.fmt(f),
            Self::Transport(error) => f.write_str(error),
        }
    }
}
impl std::error::Error for WorkspaceError {}

pub enum Workspace {
    Remote { api: Api, project_id: i64 },
    Local { handle: FileSystemDirectoryHandle },
}
impl Workspace {
    pub fn for_project(api: Api, projects: ProjectsState, id: i64) -> Option<Self> {
        match projects.project(id)?.mode {
            WorkspaceMode::Remote => Some(Self::Remote {
                api,
                project_id: id,
            }),
            WorkspaceMode::Local => projects
                .local_handles
                .with_untracked(|handles| handles.get(&id).cloned())
                .map(|handle| Self::Local { handle }),
        }
    }
    pub async fn list(&self, dir: &str) -> Result<Vec<FileEntry>, WorkspaceError> {
        let dir = workspace_path(dir)?;
        let mut entries = match self {
            Self::Remote { api, project_id } => api
                .with_value(Clone::clone)
                .list_files(*project_id, &dir)
                .await
                .map_err(WorkspaceError::from),
            Self::Local { handle } => local_fs::BrowserFsaVfs::new(handle.clone())
                .list(&dir)
                .await
                .map_err(WorkspaceError::from),
        }?;
        openwebide_core::vfs::sort_file_entries(&mut entries);
        Ok(entries)
    }
    pub async fn read(&self, path: &str) -> Result<String, WorkspaceError> {
        let path = workspace_path(path)?;
        match self {
            Self::Remote { api, project_id } => api
                .with_value(Clone::clone)
                .read_file(*project_id, &path)
                .await
                .map_err(Into::into),
            Self::Local { handle } => local_fs::BrowserFsaVfs::new(handle.clone())
                .read(&path)
                .await
                .map_err(Into::into),
        }
    }
    pub async fn read_lossy(&self, path: &str) -> Result<String, WorkspaceError> {
        let path = workspace_path(path)?;
        match self {
            Self::Remote { api, project_id } => api
                .with_value(Clone::clone)
                .read_file_lossy(*project_id, &path)
                .await
                .map_err(Into::into),
            Self::Local { handle } => local_fs::read_lossy_typed(handle, &path)
                .await
                .map_err(Into::into),
        }
    }
    pub async fn read_blob_url(&self, path: &str) -> Result<String, WorkspaceError> {
        let path = workspace_path(path)?;
        match self {
            Self::Remote { api, project_id } => api
                .with_value(Clone::clone)
                .read_file_object_url(*project_id, &path)
                .await
                .map_err(Into::into),
            Self::Local { handle } => local_fs::read_blob_url_typed(handle, &path)
                .await
                .map_err(Into::into),
        }
    }
    pub async fn write(&self, path: &str, content: &str) -> Result<(), WorkspaceError> {
        let path = workspace_path(path)?;
        match self {
            Self::Remote { api, project_id } => api
                .with_value(Clone::clone)
                .write_file(*project_id, &path, content)
                .await
                .map_err(Into::into),
            Self::Local { handle } => local_fs::BrowserFsaVfs::new(handle.clone())
                .write(&path, content)
                .await
                .map_err(Into::into),
        }
    }
    pub async fn copy(&self, from: &str, to: &str) -> Result<(), WorkspaceError> {
        let (from, to) = (workspace_path(from)?, workspace_path(to)?);
        match self {
            Self::Remote { api, project_id } => api
                .with_value(Clone::clone)
                .copy_file(*project_id, &from, &to)
                .await
                .map_err(Into::into),
            Self::Local { handle } => local_fs::BrowserFsaVfs::new(handle.clone())
                .copy(&from, &to)
                .await
                .map_err(Into::into),
        }
    }
    pub async fn create(&self, path: &str, kind: VfsEntryKind) -> Result<(), WorkspaceError> {
        let path = workspace_path(path)?;
        match self {
            Self::Remote { api, project_id } => api
                .with_value(Clone::clone)
                .create_file(*project_id, &path, kind.is_dir())
                .await
                .map_err(Into::into),
            Self::Local { handle } => local_fs::BrowserFsaVfs::new(handle.clone())
                .create(&path, kind)
                .await
                .map_err(Into::into),
        }
    }
    pub async fn search_content(
        &self,
        query: &str,
        dir: &str,
        opts: SearchOptions,
    ) -> Result<Vec<SearchHit>, WorkspaceError> {
        let dir = workspace_path(dir)?;
        match self {
            Self::Remote { api, project_id } => api
                .with_value(Clone::clone)
                .search_content(*project_id, query, &dir, opts)
                .await
                .map_err(Into::into),
            Self::Local { handle } => local_fs::BrowserFsaVfs::new(handle.clone())
                .search_content(query, &dir, opts)
                .await
                .map_err(Into::into),
        }
    }
    pub async fn delete(&self, path: &str) -> Result<(), WorkspaceError> {
        let path = workspace_path(path)?;
        match self {
            Self::Remote { api, project_id } => api
                .with_value(Clone::clone)
                .delete_file(*project_id, &path)
                .await
                .map_err(Into::into),
            Self::Local { handle } => local_fs::BrowserFsaVfs::new(handle.clone())
                .delete(&path)
                .await
                .map_err(Into::into),
        }
    }
}
