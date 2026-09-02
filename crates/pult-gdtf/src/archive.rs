//! The `.gdtf` file: a zip holding `description.xml` and the resources it names.
//!
//! Read leniently and write predictably. Reading is the untrusted direction — a
//! `.gdtf` arrives from a browser upload or from the Share, so the same guards the
//! plugin bundle reader uses apply here: an entry may not name a path outside the
//! archive, and the entry count and unpacked size are capped before anything is
//! decompressed.
//!
//! Resources are kept as raw bytes under their archive paths (`models/gltf/head.glb`,
//! `wheels/gobo1.png`, `thumbnail.png`). Nothing in this crate interprets them; the
//! browser draws the meshes and the console stores the archive whole.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::model::Gdtf;
use crate::Error;

/// The XML every `.gdtf` must hold, at the archive root.
pub const DESCRIPTION: &str = "description.xml";

/// How many entries a `.gdtf` may hold. A fixture with a mesh per geometry node and
/// a gobo image per slot is dozens; ten thousand is far above anything honest.
const MAX_ENTRIES: usize = 10_000;

/// How much it may come to unpacked. Meshes are the bulk and a detailed one is tens
/// of megabytes, so this sits well above a real file and well below what would
/// exhaust a station.
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;

/// A parsed `.gdtf`: the description, and every other file that was in the archive.
#[derive(Debug, Clone, PartialEq)]
pub struct GdtfFile {
    pub description: Gdtf,
    /// Everything else in the zip, keyed by its path inside the archive. Sorted, so
    /// writing is deterministic and two writes of the same file hash the same.
    pub resources: BTreeMap<String, Vec<u8>>,
}

impl GdtfFile {
    /// Read a `.gdtf` from bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))?;
        if archive.len() > MAX_ENTRIES {
            return Err(Error::TooLarge("this file has implausibly many entries"));
        }

        let mut description = None;
        let mut resources = BTreeMap::new();
        let mut unpacked = 0u64;

        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            // `enclosed_name` is the zip crate's own answer to zip-slip: it returns
            // `None` for an absolute path, a `..` component or a Windows drive
            // prefix. Taking the raw name instead is how an archive writes outside
            // the directory it is being unpacked into.
            let Some(path) = entry.enclosed_name() else {
                return Err(Error::BadEntry(entry.name().to_string()));
            };
            let path = path.to_string_lossy().replace('\\', "/");

            unpacked = unpacked.saturating_add(entry.size());
            if unpacked > MAX_UNPACKED_BYTES {
                return Err(Error::TooLarge("this file is implausibly large unpacked"));
            }

            let mut bytes = Vec::with_capacity(entry.size().min(1 << 20) as usize);
            entry.read_to_end(&mut bytes)?;

            if path.eq_ignore_ascii_case(DESCRIPTION) {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                description = Some(crate::xml::from_str(&text)?);
            } else {
                resources.insert(path, bytes);
            }
        }

        let description = description.ok_or(Error::NoDescription)?;
        Ok(GdtfFile {
            description,
            resources,
        })
    }

    /// Write this file back out.
    ///
    /// Deterministic: the description first, then resources in sorted order, all
    /// deflated with no timestamps. Two writes of equal input give equal bytes,
    /// which is what lets a generated file be content-addressed like a kept one.
    pub fn write(&self) -> Result<Vec<u8>, Error> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());

        writer.start_file(DESCRIPTION, options)?;
        writer.write_all(crate::xml::to_string(&self.description, "GDTF")?.as_bytes())?;

        for (path, bytes) in &self.resources {
            writer.start_file(path.as_str(), options)?;
            writer.write_all(bytes)?;
        }

        Ok(writer.finish()?.into_inner())
    }

    /// Whether these bytes look like a `.gdtf` at all.
    ///
    /// The upload route needs this: a browser sends `application/octet-stream` for a
    /// file it has no type for, so the content type is not enough to decide by.
    pub fn sniff(bytes: &[u8]) -> bool {
        let Ok(mut archive) = ZipArchive::new(Cursor::new(bytes)) else {
            return false;
        };
        (0..archive.len()).any(|index| {
            archive
                .by_index(index)
                .is_ok_and(|entry| entry.name().eq_ignore_ascii_case(DESCRIPTION))
        })
    }
}
