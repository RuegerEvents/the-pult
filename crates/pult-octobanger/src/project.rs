//! A project: the `.OBC` and the sound files it names, as they travel.
//!
//! The app keeps a controller in a folder with its two MP3s beside it, and a folder is
//! what people zip up and pass around — every project this was built against arrived as
//! one. So an import is handed either a bare `.OBC` or an archive, and the archive's
//! sound files are found **by the names the controller gives them**, in its own folder
//! first and then anywhere in the archive. Nothing is found by guessing at what a file
//! with a similar name might be.

use std::io::{Cursor, Read};

use crate::{Controller, Error};

/// The most an `.OBC` can sensibly be. The largest seen is under twenty kilobytes; a
/// controller is six thousand frames and eight names.
const MAX_CONTROLLER: u64 = 4 * 1024 * 1024;

/// The most one sound file in an archive may decompress to — the asset store's own
/// ceiling for audio, so an archive cannot hand over something a plain upload could not.
const MAX_AUDIO: u64 = 256 * 1024 * 1024;

/// What an import was handed.
pub struct Project {
    pub controllers: Vec<ProjectController>,
    archive: Option<Vec<u8>>,
}

/// One controller, and where in the upload it was.
pub struct ProjectController {
    /// Its path inside the archive, or the empty string for a bare `.OBC`.
    pub path: String,
    pub controller: Controller,
}

impl Project {
    /// Read an upload: an archive if it starts like one, a controller otherwise.
    pub fn read(bytes: &[u8]) -> Result<Project, Error> {
        if !bytes.starts_with(b"PK\x03\x04") {
            let controller = Controller::read(bytes)?;
            return Ok(Project {
                controllers: vec![ProjectController { path: String::new(), controller }],
                archive: None,
            });
        }
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Zip(e.to_string()))?;
        let mut controllers = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| Error::Zip(e.to_string()))?;
            let path = entry.name().to_string();
            if entry.is_dir() || ignored(&path) || !path.to_lowercase().ends_with(".obc") {
                continue;
            }
            let bytes = bounded(&mut entry, MAX_CONTROLLER, &path)?;
            controllers.push(ProjectController { path, controller: Controller::read(&bytes)? });
        }
        if controllers.is_empty() {
            return Err(Error::NoController);
        }
        controllers.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Project { controllers, archive: Some(bytes.to_vec()) })
    }

    /// A file a controller names, from the archive it came in. `None` where the upload
    /// was a bare `.OBC` or the archive does not carry it.
    pub fn file(&self, controller_path: &str, name: &str) -> Result<Option<Vec<u8>>, Error> {
        let Some(archive) = &self.archive else { return Ok(None) };
        let mut archive = zip::ZipArchive::new(Cursor::new(archive.as_slice())).map_err(|e| Error::Zip(e.to_string()))?;
        let folder = folder_of(controller_path);
        let wanted = name.to_lowercase();
        let mut found: Option<(bool, usize)> = None;
        for index in 0..archive.len() {
            let entry = archive.by_index(index).map_err(|e| Error::Zip(e.to_string()))?;
            let path = entry.name().to_string();
            if entry.is_dir() || ignored(&path) || file_name(&path).to_lowercase() != wanted {
                continue;
            }
            let beside = folder_of(&path) == folder;
            if found.is_none_or(|(was_beside, _)| beside && !was_beside) {
                found = Some((beside, index));
            }
        }
        let Some((_, index)) = found else { return Ok(None) };
        let mut entry = archive.by_index(index).map_err(|e| Error::Zip(e.to_string()))?;
        let path = entry.name().to_string();
        Ok(Some(bounded(&mut entry, MAX_AUDIO, &path)?))
    }
}

/// What a Mac puts in an archive beside every file, which is never the file.
fn ignored(path: &str) -> bool {
    path.starts_with("__MACOSX/") || file_name(path).starts_with("._")
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn folder_of(path: &str) -> &str {
    path.rfind(['/', '\\']).map(|at| &path[..at]).unwrap_or("")
}

/// Read an entry, refusing one that decompresses past `max` rather than finding out
/// how large it really is.
fn bounded(entry: &mut impl Read, max: u64, path: &str) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    entry.take(max + 1).read_to_end(&mut bytes).map_err(|e| Error::Zip(e.to_string()))?;
    if bytes.len() as u64 > max {
        return Err(Error::TooLarge(path.to_string()));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_has_a_folder_and_a_name() {
        assert_eq!(folder_of("donkey-main/Donkey.OBC"), "donkey-main");
        assert_eq!(folder_of("Donkey.OBC"), "");
        assert_eq!(file_name("a/b/Donkey_Trigger.mp3"), "Donkey_Trigger.mp3");
        assert!(ignored("__MACOSX/a/._Donkey.OBC"));
        assert!(ignored("a/._Donkey.OBC"));
        assert!(!ignored("a/Donkey.OBC"));
    }
}
