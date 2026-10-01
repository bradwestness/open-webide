use openwebide_core::file_type::extension;

/// Infer MIME content type from file path extension.
pub fn mime_type_from_path(path: &str) -> &'static str {
    let ext = extension(path).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "txt" | "md" | "rs" | "py" | "toml" | "yaml" | "yml" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mime_types() {
        assert_eq!(mime_type_from_path("dir/FILE.PNG"), "image/png");
        assert_eq!(mime_type_from_path("file.svg"), "image/svg+xml");
        assert_eq!(mime_type_from_path("file.rs"), "text/plain; charset=utf-8");
        assert_eq!(mime_type_from_path("unknown"), "application/octet-stream");
    }
}
