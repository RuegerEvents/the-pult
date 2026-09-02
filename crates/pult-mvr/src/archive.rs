//! The `.mvr` file: a zip holding `GeneralSceneDescription.xml`, the GDTF files the
//! fixtures in it are, and the meshes and textures the objects in it are drawn with.
//!
//! Read leniently and write predictably, with the same guards the GDTF reader uses:
//! an entry may not name a path outside the archive, and the entry count and unpacked
//! size are capped before anything is decompressed.
//!
//! Everything but the scene description is kept as raw bytes under its archive name.
//! Nothing here interprets a `.gdtf`, a `.glb` or a `.3ds` — the console stores them
//! and the browser draws them.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::model::GeneralSceneDescription;
use crate::{Error, Warning};

/// The XML every `.mvr` must hold, at the archive root.
pub const SCENE_DESCRIPTION: &str = "GeneralSceneDescription.xml";

/// How many entries an `.mvr` may hold. A big rig is a GDTF per type and a mesh per
/// truss part: a hundred files is normal, ten thousand is not.
const MAX_ENTRIES: usize = 10_000;

/// How much it may come to unpacked. A real 9 MB archive unpacks to about 20 MB; a
/// venue-sized one with detailed meshes is larger again, and this sits well above
/// that and well below what would exhaust a station.
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;

/// A parsed `.mvr`: the scene, and every other file that was in the archive.
#[derive(Debug, Clone, PartialEq)]
pub struct MvrFile {
    pub scene: GeneralSceneDescription,
    /// Everything else in the zip, keyed by its name inside the archive. Sorted, so
    /// writing is deterministic and two writes of the same file hash the same.
    pub resources: BTreeMap<String, Vec<u8>>,
    /// What had to be forgiven to read it. Never an error: a file that needed
    /// forgiving is still a rig somebody has to patch tonight.
    pub warnings: Vec<Warning>,
}

impl MvrFile {
    /// Read an `.mvr` from bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))?;
        if archive.len() > MAX_ENTRIES {
            return Err(Error::TooLarge("this file has implausibly many entries"));
        }

        let mut description = None;
        let mut resources = BTreeMap::new();
        let mut warnings = Vec::new();
        let mut unpacked = 0u64;

        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let Some(path) = entry.enclosed_name() else {
                return Err(Error::BadEntry(entry.name().to_string()));
            };
            let name = path.to_string_lossy().replace('\\', "/");

            unpacked = unpacked.saturating_add(entry.size());
            if unpacked > MAX_UNPACKED_BYTES {
                return Err(Error::TooLarge("this file unpacks to implausibly much"));
            }

            let mut buffer = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buffer)?;

            if name.eq_ignore_ascii_case(SCENE_DESCRIPTION) {
                let text = String::from_utf8_lossy(&buffer).into_owned();
                description = Some(read_scene(&text, &mut warnings)?);
            } else {
                resources.insert(name, buffer);
            }
        }

        let Some(scene) = description else {
            return Err(Error::NoSceneDescription);
        };
        Ok(MvrFile {
            scene,
            resources,
            warnings,
        })
    }

    /// Write an `.mvr`.
    pub fn write(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

            zip.start_file(SCENE_DESCRIPTION, options)?;
            let xml = pult_gdtf::xml::to_string(&self.scene, "GeneralSceneDescription")?;
            zip.write_all(xml.as_bytes())?;

            for (name, bytes) in &self.resources {
                zip.start_file(name.as_str(), options)?;
                zip.write_all(bytes)?;
            }
            zip.finish()?;
        }
        Ok(buffer.into_inner())
    }

    /// The archive entry a `GDTFSpec` names, under whichever of the spellings the
    /// file that named it used.
    ///
    /// Three real exporters write three different things. grandMA writes
    /// `Vendor@Product` with no extension; Vectorworks writes `Vendor@Product.gdtf`
    /// with one. And a zip's central directory does not always decode an entry name
    /// the way the XML spells it — an ARRI Orbiter whose product name carries a
    /// degree sign is in the corpus for exactly that. So the match walks down from
    /// exact, and the caller is told which rung answered so it can say so.
    pub fn gdtf_named(&self, spec: &str) -> Option<(&str, &[u8], SpecMatch)> {
        let wanted = spec.trim();

        for (rung, matches) in [
            (SpecMatch::Exact, exact as fn(&str, &str) -> bool),
            (SpecMatch::Extension, ignoring_extension),
            (SpecMatch::Case, ignoring_case),
            (SpecMatch::Loosely, loosely),
        ] {
            if let Some((name, bytes)) = self
                .resources
                .iter()
                .filter(|(name, _)| name.to_ascii_lowercase().ends_with(".gdtf"))
                .find(|(name, _)| matches(name, wanted))
            {
                return Some((name.as_str(), bytes.as_slice(), rung));
            }
        }
        None
    }
}

/// Which rung of [`MvrFile::gdtf_named`]'s ladder found the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecMatch {
    /// The name in the XML is the name in the archive.
    Exact,
    /// The same, once `.gdtf` is discounted.
    Extension,
    /// The same but for capitals.
    Case,
    /// The same once everything that is not a letter or a digit is discounted, which
    /// is what it takes when a zip's entry name and the XML disagree about a
    /// non-ASCII character.
    Loosely,
}

fn exact(entry: &str, wanted: &str) -> bool {
    entry == wanted
}

/// Without the extension on either side, which is the usual case: grandMA writes the
/// spec bare and the archive entry has to have the suffix.
fn ignoring_extension(entry: &str, wanted: &str) -> bool {
    bare(entry) == bare(wanted)
}

fn ignoring_case(entry: &str, wanted: &str) -> bool {
    bare(entry).eq_ignore_ascii_case(bare(wanted))
}

/// Letters and digits only. What it takes when a zip's central directory and the XML
/// disagree about a character — an ARRI Orbiter whose name carries a degree sign
/// comes out of the archive as `15┬░` and out of the XML as `15°`.
///
/// The last rung, and deliberately the last: it would also match two fixtures whose
/// names differ only in punctuation, so a caller that lands here says so.
fn loosely(entry: &str, wanted: &str) -> bool {
    fn letters(text: &str) -> String {
        bare(text)
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }
    let (a, b) = (letters(entry), letters(wanted));
    !a.is_empty() && a == b
}

fn bare(name: &str) -> &str {
    name.strip_suffix(".gdtf").unwrap_or(name)
}

/// Parse the scene description, forgiving what real files put around it.
fn read_scene(text: &str, warnings: &mut Vec<Warning>) -> Result<GeneralSceneDescription, Error> {
    let trimmed = text.trim_end_matches(['\0', ' ', '\n', '\r', '\t']);
    if trimmed.len() != text.trim_end_matches([' ', '\n', '\r', '\t']).len() {
        // A real grandMA export ends with a NUL byte after the closing tag. Every
        // strict XML parser refuses the whole document over it — which is a rig
        // nobody can open because of one byte nothing reads.
        warnings.push(Warning::new(
            SCENE_DESCRIPTION,
            "the scene description has bytes after its closing tag; they were ignored",
        ));
    }
    Ok(pult_gdtf::xml::from_str(trimmed)?)
}
