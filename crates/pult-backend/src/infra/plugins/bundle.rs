//! A plugin as one file: the zip a show carries and a station unpacks.
//!
//! The unit that has to replicate is a manifest *plus* a component *plus* the
//! scripts a panel loads. Hashing the component alone would leave the other two
//! unversioned, so the digest is taken over the archive and the archive is the
//! plugin.
//!
//! **This is the one place in the console that opens an archive somebody else
//! made**, and it is written accordingly: an entry may not name a path outside
//! the directory it is being written into, may not be a symlink, and the whole
//! bundle is capped in entry count and in uncompressed size. A zip is a
//! description of where to write files, and taking one on trust is how a
//! directory somewhere else gets overwritten.
//!
//! Extraction is deliberately not atomic-on-failure at the file level: it unpacks
//! into a temporary directory and renames it into place only once everything has
//! landed, so a half-unpacked bundle is never visible as a plugin.

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::manifest::PluginManifest;

/// The manifest's name inside a bundle, and on disk once unpacked.
pub const MANIFEST_NAME: &str = "pult-plugin.toml";

/// How many files a bundle may hold. A plugin is a manifest, a component and a
/// handful of scripts; a thousand is far above anything honest and far below
/// anything that could exhaust a station.
const MAX_ENTRIES: usize = 1_000;

/// How much a bundle may come to once unpacked. The compressed ceiling is the
/// asset store's; this is the one that matters, because a zip can claim to be
/// very small and not be.
const MAX_UNPACKED_BYTES: u64 = 128 * 1024 * 1024;

/// What a bundle says about itself, read without unpacking it.
pub struct BundleInfo {
    pub manifest: PluginManifest,
}

/// Read the manifest out of a bundle, and nothing else.
///
/// This is what the install path uses: a bundle that is not a plugin should be
/// refused before its bytes are stored, so that a rejected upload leaves nothing
/// behind. The manifest is parsed against `dir`, which is where it *would* be
/// unpacked, so relative paths in it validate the same way they will later.
pub fn read_manifest(bytes: &[u8], dir: &Path) -> Result<BundleInfo> {
    let mut archive = open(bytes)?;
    let mut file = archive
        .by_name(MANIFEST_NAME)
        .with_context(|| format!("a bundle must contain a {MANIFEST_NAME} at its root"))?;
    if file.size() > MAX_UNPACKED_BYTES {
        bail!("the manifest in this bundle is implausibly large");
    }
    let mut text = String::new();
    file.read_to_string(&mut text).context("reading the manifest")?;

    let manifest = PluginManifest::parse(dir, &text).map_err(|e| anyhow::anyhow!(e))?;
    Ok(BundleInfo { manifest })
}

/// Unpack a bundle into `dir`, which must not already exist.
///
/// Returns the parsed manifest, so a caller that has just unpacked one does not
/// have to read it off the disk again.
pub fn extract(bytes: &[u8], dir: &Path) -> Result<PluginManifest> {
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    let parent = dir.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;

    // Unpack beside the destination and rename in, so nothing ever sees a
    // directory that is half a plugin. The suffix carries the process id and the
    // destination's own name, so two stations on one machine unpacking the same
    // digest at the same moment do not land in each other's staging.
    let staging = parent.join(format!(
        ".{}.unpacking.{}",
        dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&staging);

    let result = unpack_into(bytes, &staging);
    match result {
        Ok(()) => {}
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    }

    match std::fs::rename(&staging, dir) {
        Ok(()) => {}
        Err(_) if dir.is_dir() => {
            // Somebody else won the race and put the same digest there. Their
            // copy is byte-identical to ours by definition, so theirs will do.
            let _ = std::fs::remove_dir_all(&staging);
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e).context("moving the unpacked bundle into place");
        }
    }

    let text = std::fs::read_to_string(dir.join(MANIFEST_NAME))
        .with_context(|| format!("a bundle must contain a {MANIFEST_NAME} at its root"))?;
    let manifest = PluginManifest::parse(dir, &text).map_err(|e| anyhow::anyhow!(e))?;

    // The manifest names a component; a bundle that does not carry it is not a
    // plugin, and finding that out now is better than finding it out at load.
    if !manifest.wasm_path().is_file() {
        bail!("the bundle's manifest names {:?}, which is not in it", manifest.plugin.wasm);
    }
    Ok(manifest)
}

fn open(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>> {
    let archive = zip::ZipArchive::new(Cursor::new(bytes)).context("this is not a readable zip")?;
    if archive.len() > MAX_ENTRIES {
        bail!("a bundle may hold {MAX_ENTRIES} files; this one holds {}", archive.len());
    }
    Ok(archive)
}

fn unpack_into(bytes: &[u8], staging: &Path) -> Result<()> {
    let mut archive = open(bytes)?;
    std::fs::create_dir_all(staging)?;

    let mut unpacked: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;

        // `enclosed_name` is None for anything that would escape — absolute
        // paths, `..` components, and Windows drive letters and UNC prefixes,
        // which matter because a showfile is portable between platforms.
        let Some(relative) = entry.enclosed_name() else {
            bail!("the bundle holds an entry named {:?}, which would write outside it", entry.name());
        };
        let relative = checked(&relative)
            .with_context(|| format!("the bundle holds an entry named {:?}", entry.name()))?;

        // A symlink is a path written into a file, so nothing above catches one:
        // the entry's own name is innocent and its *contents* point elsewhere.
        // Following it later would write through it.
        if is_symlink(&entry) {
            bail!("the bundle holds a symlink ({}), which a plugin may not carry", relative.display());
        }

        let target = staging.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }

        if unpacked.saturating_add(entry.size()) > MAX_UNPACKED_BYTES {
            bail!("this bundle unpacks to more than the {MAX_UNPACKED_BYTES} bytes one may");
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Copied through a capped reader as well as being checked above, because
        // the size in the header is a claim and the bytes are the fact. One byte
        // over the remaining budget is enough to tell the two apart.
        let budget = MAX_UNPACKED_BYTES - unpacked;
        let mut out = std::fs::File::create(&target)
            .with_context(|| format!("writing {}", relative.display()))?;
        let written = std::io::copy(&mut entry.by_ref().take(budget + 1), &mut out)
            .with_context(|| format!("writing {}", relative.display()))?;
        if written > budget {
            bail!("this bundle unpacks to more than the {MAX_UNPACKED_BYTES} bytes one may");
        }
        unpacked += written;
    }
    Ok(())
}

/// A path that is relative, has no `..` in it, and names something.
fn checked(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("with an empty name");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => bail!("which is not a plain relative path"),
        }
    }
    Ok(path.to_path_buf())
}

/// Unix mode bits say what an entry is; the top four say symlink.
fn is_symlink(entry: &zip::read::ZipFile<'_>) -> bool {
    entry.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000)
}

#[cfg(test)]
mod tests;
