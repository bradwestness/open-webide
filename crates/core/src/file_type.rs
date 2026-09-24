//! File classification and preview metadata.

use serde::{Deserialize, Serialize};

/// Extract the lowercase extension from a file path: the text after the last
/// `.` in the final path segment. Returns `None` when the segment has no dot,
/// the dot is first (`.bashrc`), or the extension is empty (`file.`).
pub fn extension(path: &str) -> Option<String> {
    let segment = path.rsplit('/').next()?;
    let (stem, ext) = segment.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// The high-level kind of a file, determined from its extension or content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    /// Editable text or source code.
    Text,
    /// Markdown documentation with rich HTML preview.
    Markdown,
    /// Image asset rendered visually (PNG, JPG, SVG, WebP, GIF, ICO, BMP, AVIF).
    Image,
    /// Audio / video media asset.
    Media,
    /// Binary file or archive (WASM, PDF, ZIP, TAR, DB, etc.) not directly editable as text.
    Binary,
}

impl FileKind {
    /// Categorize a file by its path extension.
    pub fn from_path(path: &str) -> Self {
        let ext = extension(path).unwrap_or_default();
        match ext.as_str() {
            "md" | "markdown" | "mdown" | "mkd" => FileKind::Markdown,
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" | "avif" | "tiff" => {
                FileKind::Image
            }
            "mp3" | "wav" | "ogg" | "m4a" | "flac" | "mp4" | "webm" | "mov" | "avi" | "mkv" => {
                FileKind::Media
            }
            "wasm" | "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "iso" | "dmg"
            | "pdf" | "bin" | "exe" | "dll" | "so" | "dylib" | "o" | "a" | "obj" | "pyc"
            | "class" | "jar" | "db" | "sqlite" | "sqlite3" | "woff" | "woff2" | "ttf" | "otf"
            | "eot" => FileKind::Binary,
            _ => FileKind::Text,
        }
    }

    /// Whether this file is non-text and should default to Preview / Placeholder.
    pub fn is_non_text(&self) -> bool {
        matches!(self, FileKind::Image | FileKind::Media | FileKind::Binary)
    }

    /// Whether this file is previewable directly in the editor (Markdown or Image).
    pub fn is_previewable(&self) -> bool {
        matches!(self, FileKind::Markdown | FileKind::Image)
    }

    /// A friendly descriptive label for the file type.
    pub fn description(&self, path: &str) -> &'static str {
        let ext = extension(path).unwrap_or_default();
        match ext.as_str() {
            "wasm" => "WebAssembly Component / Binary",
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" => "Compressed Archive",
            "pdf" => "PDF Document",
            "db" | "sqlite" | "sqlite3" => "SQLite Database",
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" | "avif" => {
                "Image Asset"
            }
            "mp3" | "wav" | "ogg" | "m4a" | "flac" => "Audio File",
            "mp4" | "webm" | "mov" | "avi" | "mkv" => "Video File",
            "woff" | "woff2" | "ttf" | "otf" | "eot" => "Font Asset",
            "exe" | "dll" | "so" | "dylib" => "Compiled Native Binary",
            _ => match self {
                FileKind::Image => "Image Asset",
                FileKind::Media => "Media File",
                FileKind::Binary => "Binary File",
                FileKind::Markdown => "Markdown Document",
                FileKind::Text => "Text / Source Code",
            },
        }
    }

    /// Icon glyph for the file type.
    pub fn glyph(&self, path: &str) -> &'static str {
        let ext = extension(path).unwrap_or_default();
        match ext.as_str() {
            "wasm" => "⚙️",
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" => "📦",
            "pdf" => "📑",
            "db" | "sqlite" | "sqlite3" => "🗄️",
            "mp3" | "wav" | "ogg" | "m4a" | "flac" => "🎵",
            "mp4" | "webm" | "mov" | "avi" | "mkv" => "🎬",
            "woff" | "woff2" | "ttf" | "otf" | "eot" => "🔤",
            "exe" | "dll" | "so" | "dylib" => "⚡",
            _ => match self {
                FileKind::Image => "🖼️",
                FileKind::Media => "🎬",
                FileKind::Binary => "📦",
                FileKind::Markdown => "📝",
                FileKind::Text => "📄",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extension_table() {
        // A bare word has no extension, so it stays text even when it
        // matches a known binary extension name.
        assert_eq!(FileKind::from_path("bin"), FileKind::Text);
        assert_eq!(FileKind::from_path("db"), FileKind::Text);
        assert_eq!(FileKind::from_path("a"), FileKind::Text);
        // Dots in directory segments are not extensions.
        assert_eq!(extension("v1.2/Makefile"), None);
        // A leading dot (dotfile) is not an extension.
        assert_eq!(extension(".bashrc"), None);
        // The extension is the text after the last dot, lowercased.
        assert_eq!(extension("x.TAR.GZ"), Some("gz".to_string()));
    }

    #[test]
    fn test_file_kind_classification() {
        assert_eq!(FileKind::from_path("README.md"), FileKind::Markdown);
        assert_eq!(
            FileKind::from_path("docs/spec.markdown"),
            FileKind::Markdown
        );
        assert_eq!(FileKind::from_path("src/lib.rs"), FileKind::Text);
        assert_eq!(FileKind::from_path("main.py"), FileKind::Text);
        assert_eq!(FileKind::from_path("index.html"), FileKind::Text);

        assert_eq!(FileKind::from_path("assets/logo.png"), FileKind::Image);
        assert_eq!(FileKind::from_path("icon.svg"), FileKind::Image);
        assert_eq!(FileKind::from_path("photo.jpeg"), FileKind::Image);

        assert_eq!(FileKind::from_path("target/backend.wasm"), FileKind::Binary);
        assert_eq!(FileKind::from_path("archive.zip"), FileKind::Binary);
        assert_eq!(FileKind::from_path("document.pdf"), FileKind::Binary);
        assert_eq!(FileKind::from_path("data.sqlite3"), FileKind::Binary);

        assert!(FileKind::from_path("logo.png").is_non_text());
        assert!(FileKind::from_path("backend.wasm").is_non_text());
        assert!(!FileKind::from_path("src/lib.rs").is_non_text());
        assert!(!FileKind::from_path("README.md").is_non_text());

        assert!(FileKind::from_path("README.md").is_previewable());
        assert!(FileKind::from_path("logo.png").is_previewable());
        assert!(!FileKind::from_path("backend.wasm").is_previewable());
    }
}
