use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum SecretSource {
    Env,
    File(PathBuf),
}

pub fn load_or_create(
    env: Option<String>,
    file: Option<PathBuf>,
    home: Option<PathBuf>,
    xdg: Option<PathBuf>,
) -> Result<(String, SecretSource), String> {
    if let Some(s) = env {
        if s.len() < 32 {
            return Err("OPENWEBIDE_BRIDGE_SECRET must be at least 32 characters".to_string());
        }
        return Ok((s, SecretSource::Env));
    }

    let file_path = if let Some(p) = file {
        p
    } else if let Some(x) = xdg {
        x.join("openwebide").join("bridge-secret")
    } else if let Some(h) = home {
        h.join(".config").join("openwebide").join("bridge-secret")
    } else {
        return Err("Cannot determine bridge secret file path (no --secret-file, no $XDG_CONFIG_HOME, no $HOME)".to_string());
    };

    if file_path.exists() {
        let content = fs::read_to_string(&file_path)
            .map_err(|e| format!("failed to read secret file {}: {}", file_path.display(), e))?;
        let s = content.trim().to_string();
        if s.len() < 32 {
            return Err(format!(
                "secret in {} must be at least 32 characters",
                file_path.display()
            ));
        }
        return Ok((s, SecretSource::File(file_path)));
    }

    if let Some(parent) = file_path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create directory {}: {}", parent.display(), e))?;
    }

    let mut rand_bytes = [0u8; 32];
    use rand::Rng;
    rand::rng().fill_bytes(&mut rand_bytes);
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let secret = URL_SAFE_NO_PAD.encode(rand_bytes);

    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    opts.mode(0o600);

    let mut f = opts.open(&file_path).map_err(|e| {
        format!(
            "failed to create secret file {}: {}",
            file_path.display(),
            e
        )
    })?;

    use std::io::Write;
    f.write_all(secret.as_bytes())
        .map_err(|e| format!("failed to write secret file {}: {}", file_path.display(), e))?;

    Ok((secret, SecretSource::File(file_path)))
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut res = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        res |= x ^ y;
    }
    res == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_load_or_create_env() {
        let s = "01234567890123456789012345678901".to_string();
        assert_eq!(
            load_or_create(Some(s.clone()), None, None, None),
            Ok((s, SecretSource::Env))
        );
        let short = "short".to_string();
        assert!(load_or_create(Some(short), None, None, None).is_err());
    }

    #[test]
    fn test_load_or_create_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secret");

        // Creation
        let (s1, src1) = load_or_create(None, Some(path.clone()), None, None).unwrap();
        assert_eq!(src1, SecretSource::File(path.clone()));
        assert!(s1.len() >= 32);

        // Reuse
        let (s2, src2) = load_or_create(None, Some(path.clone()), None, None).unwrap();
        assert_eq!(src2, SecretSource::File(path.clone()));
        assert_eq!(s1, s2);
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
    }
}
