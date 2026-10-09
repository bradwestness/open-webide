//! Portable skill file/folder/archive import and ZIP export, shared by every host.
//! Decode in memory: no filesystem extraction, process spawning or script execution.
use super::{MAX_RESOURCES, MAX_SKILL_BYTES, SkillDraft, SkillResource, validate_resource_name};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
};

pub const MAX_ARCHIVE_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 128;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

fn archive_name(raw: &str, directory: bool) -> Result<Option<String>, String> {
    let mut name = raw;
    while let Some(relative) = name.strip_prefix("./") {
        name = relative;
    }
    let name = if directory {
        name.trim_end_matches('/')
    } else {
        name
    };
    if directory && matches!(name, "" | ".") {
        return Ok(None);
    }
    validate_resource_name(name)?;
    Ok(Some(name.to_string()))
}
fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read skill content: {error}"))?;
    if bytes.len() > limit {
        return Err("Skill import exceeds its content limit".into());
    }
    Ok(bytes)
}
/// A picker provides relative names and bytes; the same validator handles archives.
pub fn import_files(files: Vec<ImportFile>) -> Result<SkillDraft, String> {
    if files.is_empty() || files.len() > MAX_RESOURCES + 1 {
        return Err("Choose one skill with at most 16 supporting files".into());
    }
    let mut entries = BTreeMap::new();
    let mut total = 0usize;
    for file in files {
        validate_resource_name(&file.name)?;
        total = total.saturating_add(file.name.len() + file.bytes.len());
        if total > MAX_SKILL_BYTES {
            return Err("Skill content is limited to 128 KiB".into());
        }
        if entries.insert(file.name, file.bytes).is_some() {
            return Err("Duplicate file name in skill import".into());
        }
    }
    let manifests = entries
        .keys()
        .filter(|name| name.rsplit('/').next() == Some("SKILL.md"))
        .cloned()
        .collect::<Vec<_>>();
    if manifests.len() != 1 {
        return Err("Choose a folder or archive containing exactly one SKILL.md".into());
    }
    let manifest = &manifests[0];
    let prefix = manifest.strip_suffix("SKILL.md").expect("manifest suffix");
    if entries.keys().any(|name| !name.starts_with(prefix)) {
        return Err("All files must belong to the skill folder".into());
    }
    let text =
        std::str::from_utf8(&entries[manifest]).map_err(|_| "SKILL.md must contain UTF-8 text")?;
    let mut draft = import_markdown(text)?;
    for (name, bytes) in &entries {
        if name == manifest {
            continue;
        }
        let name = name
            .strip_prefix(prefix)
            .expect("validated prefix")
            .to_string();
        let (content, binary) = match String::from_utf8(bytes.clone()) {
            Ok(text) => (text, false),
            Err(_) => (STANDARD.encode(bytes), true),
        };
        draft.resources.push(SkillResource {
            name,
            content,
            binary,
        });
    }
    draft.validate()?;
    Ok(draft)
}
/// Standard SKILL.md YAML frontmatter; preserve additional portable metadata.
pub fn import_markdown(text: &str) -> Result<SkillDraft, String> {
    if text.len() > MAX_SKILL_BYTES {
        return Err("Skill file exceeds 128 KiB".into());
    }
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n");
    let body = text
        .strip_prefix("---\n")
        .ok_or("SKILL.md needs YAML frontmatter with name and description")?;
    let (frontmatter, instructions) = body
        .split_once("\n---\n")
        .ok_or("SKILL.md frontmatter is not closed")?;
    let mut metadata: BTreeMap<String, serde_json::Value> = serde_yaml_ng::from_str(frontmatter)
        .map_err(|error| format!("Invalid skill frontmatter: {error}"))?;
    let name = metadata
        .remove("name")
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or("Skill frontmatter needs a string name")?;
    let description = metadata
        .remove("description")
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or("Skill frontmatter needs a string description")?;
    let draft = SkillDraft {
        name,
        description,
        instructions: instructions.trim().into(),
        enabled: true,
        resources: Vec::new(),
        metadata,
    };
    draft.validate()?;
    Ok(draft)
}
pub fn markdown(draft: &SkillDraft) -> Result<String, String> {
    draft.validate()?;
    let mut fields = draft.metadata.clone();
    fields.insert("name".into(), serde_json::json!(draft.name));
    fields.insert("description".into(), serde_json::json!(draft.description));
    let yaml = serde_yaml_ng::to_string(&fields).map_err(|error| error.to_string())?;
    Ok(format!("---\n{yaml}---\n\n{}\n", draft.instructions))
}
/// Formats are detected by the selected filename; decoding errors never fall back to text.
pub fn import_archive(name: &str, bytes: &[u8]) -> Result<SkillDraft, String> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("Skill archive is limited to 1 MiB".into());
    }
    let name = name.to_ascii_lowercase();
    let extension = name.rsplit('.').next().unwrap_or_default();
    let files = if matches!(extension, "zip" | "skill") {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|error| format!("Invalid ZIP: {error}"))?;
        if archive.len() > MAX_ENTRIES {
            return Err("Too many archive entries".into());
        }
        let mut files = Vec::new();
        let mut remaining = MAX_SKILL_BYTES;
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .map_err(|error| format!("Cannot read ZIP entry: {error}"))?;
            let name = archive_name(entry.name(), entry.is_dir())?;
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err("Skill archives cannot contain links".into());
            }
            if entry.is_dir() {
                continue;
            }
            let name = name.ok_or("Archive file has no name")?;
            let bytes = read_bounded(entry, remaining)?;
            remaining = remaining.saturating_sub(bytes.len());
            files.push(ImportFile { name, bytes });
        }
        files
    } else if name.ends_with(".tar.gz") || extension == "tgz" {
        // Bound the whole tar stream including padding and extended headers.
        let tar = read_bounded(flate2::read::GzDecoder::new(bytes), MAX_ARCHIVE_BYTES)?;
        tar_files(&tar)?
    } else if extension == "tar" {
        tar_files(bytes)?
    } else {
        return Err("Choose SKILL.md, ZIP/.skill, tar, tar.gz or tgz".into());
    };
    import_files(files)
}
fn tar_files(bytes: &[u8]) -> Result<Vec<ImportFile>, String> {
    let mut archive = tar::Archive::new(bytes);
    let mut files = Vec::new();
    let mut remaining = MAX_SKILL_BYTES;
    for (index, entry) in archive
        .entries()
        .map_err(|error| error.to_string())?
        .enumerate()
    {
        if index >= MAX_ENTRIES {
            return Err("Too many archive entries".into());
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry
            .path()
            .map_err(|error| error.to_string())?
            .to_str()
            .ok_or("Archive names must be UTF-8")?
            .to_string();
        let name = archive_name(&name, entry.header().entry_type().is_dir())?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err("Skill archives can contain only regular files and directories".into());
        }
        let name = name.ok_or("Archive file has no name")?;
        let bytes = read_bounded(entry, remaining)?;
        remaining = remaining.saturating_sub(bytes.len());
        files.push(ImportFile { name, bytes });
    }
    Ok(files)
}
pub fn export_zip(draft: &SkillDraft) -> Result<Vec<u8>, String> {
    draft.validate()?;
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    archive
        .start_file(format!("{}/SKILL.md", draft.name), options)
        .map_err(|error| error.to_string())?;
    archive
        .write_all(markdown(draft)?.as_bytes())
        .map_err(|error| error.to_string())?;
    for resource in &draft.resources {
        archive
            .start_file(format!("{}/{}", draft.name, resource.name), options)
            .map_err(|error| error.to_string())?;
        let bytes = if resource.binary {
            STANDARD
                .decode(&resource.content)
                .map_err(|error| error.to_string())?
        } else {
            resource.content.as_bytes().to_vec()
        };
        archive
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
    }
    archive
        .finish()
        .map(Cursor::into_inner)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draft() -> SkillDraft {
        import_markdown("---\nname: build-check\ndescription: >-\n  Check build failures when asked to validate changes.\nlicense: MIT\nmetadata:\n  owner: test\n---\n\nRead references/check.md, then run the approved checks. 🦀\n").unwrap()
    }
    #[test]
    fn skill_zip_round_trip_preserves_metadata_text_binary_and_nested_resources() {
        let mut skill = draft();
        skill.resources = vec![
            SkillResource {
                name: "references/check.md".into(),
                content: "Use cargo test 🦀".into(),
                binary: false,
            },
            SkillResource {
                name: "assets/icon.bin".into(),
                content: STANDARD.encode([0, 255, 128, 1]),
                binary: true,
            },
        ];
        skill.resources.sort_by(|a, b| a.name.cmp(&b.name));
        let bytes = export_zip(&skill).unwrap();
        assert_eq!(import_archive("skill.zip", &bytes).unwrap(), skill);
        assert_eq!(import_archive("skill.skill", &bytes).unwrap(), skill);
        assert_eq!(
            import_files(vec![ImportFile {
                name: "SKILL.md".into(),
                bytes: markdown(&draft()).unwrap().into_bytes()
            }])
            .unwrap(),
            draft()
        );
    }
    fn tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *bytes).unwrap();
        }
        builder.into_inner().unwrap()
    }
    #[test]
    fn skill_tar_and_gzip_import_use_the_same_folder_contract() {
        let skill = draft();
        let manifest = markdown(&skill).unwrap();
        let archive = tar(&[("build-check/SKILL.md", manifest.as_bytes())]);
        assert_eq!(import_archive("skill.tar", &archive).unwrap(), skill);
        let relative = tar(&[("./SKILL.md", manifest.as_bytes())]);
        assert_eq!(import_archive("relative.tar", &relative).unwrap(), skill);
        assert!(archive_name("./../outside", false).is_err());
        assert_eq!(archive_name("./", true).unwrap(), None);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&archive).unwrap();
        let gzip = encoder.finish().unwrap();
        for name in ["skill.tgz", "skill.tar.gz"] {
            assert_eq!(import_archive(name, &gzip).unwrap(), skill);
        }
        assert!(import_archive("skill.tar.gz", b"broken").is_err());
    }
    #[test]
    fn skill_import_rejects_traversal_duplicates_multiple_roots_links_and_bombs() {
        let manifest = markdown(&draft()).unwrap();
        for name in [
            "../SKILL.md",
            "/SKILL.md",
            "C:/SKILL.md",
            "a/../SKILL.md",
            "a\\SKILL.md",
        ] {
            assert!(
                import_files(vec![ImportFile {
                    name: name.into(),
                    bytes: manifest.as_bytes().to_vec()
                }])
                .is_err(),
                "{name}"
            );
        }
        assert!(
            import_files(vec![
                ImportFile {
                    name: "SKILL.md".into(),
                    bytes: manifest.as_bytes().to_vec()
                },
                ImportFile {
                    name: "SKILL.md".into(),
                    bytes: manifest.as_bytes().to_vec()
                }
            ])
            .is_err()
        );
        assert!(
            import_files(vec![
                ImportFile {
                    name: "one/SKILL.md".into(),
                    bytes: manifest.as_bytes().to_vec()
                },
                ImportFile {
                    name: "two/SKILL.md".into(),
                    bytes: manifest.as_bytes().to_vec()
                }
            ])
            .is_err()
        );
        assert!(
            import_files(vec![
                ImportFile {
                    name: "one/SKILL.md".into(),
                    bytes: manifest.as_bytes().to_vec()
                },
                ImportFile {
                    name: "outside.txt".into(),
                    bytes: Vec::new()
                }
            ])
            .is_err()
        );
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            "SKILL.md",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(&vec![b'a'; MAX_SKILL_BYTES + 1]).unwrap();
        assert!(import_archive("bomb.zip", &zip.finish().unwrap().into_inner()).is_err());
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.add_symlink("link", "SKILL.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        assert!(import_archive("link.zip", &zip.finish().unwrap().into_inner()).is_err());
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("SKILL.md").unwrap();
        header.set_cksum();
        builder.append_data(&mut header, "link", &[][..]).unwrap();
        assert!(import_archive("link.tar", &builder.into_inner().unwrap()).is_err());
        assert!(import_archive("unsupported.rar", b"data").is_err());
        assert!(import_markdown("---\nname: wrong\n---\ntext").is_err());
    }
}
