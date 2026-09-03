//! A show that has to get somewhere: `.pultz`, which is the bundle zipped.
//!
//! A folder is the right shape for a show a console has open — the assets are files
//! the snapshots can share, and an operator can drag the whole of it onto a stick.
//! It is the wrong shape for everything else. A folder does not go in an email, does
//! not survive most upload forms, and on some platforms is not one thing at all. So
//! the travelling form is a single file, and it is a zip of exactly the folder.
//!
//! Two decisions worth stating.
//!
//! **The database is copied out, not read off the disk.** A show that is open has a
//! `-wal` beside it holding writes the main file does not have yet, and zipping the
//! three of them would hand somebody a show that opens with the last few minutes
//! missing or refuses to open at all. `VACUUM INTO` writes one whole compacted
//! database, which is also what makes an exported show smaller than the one it
//! came from.
//!
//! **The versions stay behind unless they are asked for.** They are the bulk of a
//! bundle by a long way — one whole database each — and somebody sending a show to a
//! colleague is sending the show, not their afternoon's undo history. `?versions=1`
//! says otherwise.

use std::{
    io::{Cursor, Read, Write},
    path::Path,
};

use anyhow::{bail, Context, Result};
use sqlx::SqlitePool;
use tracing::{info, warn};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use super::bundle::{self, Bundle};

/// What a `.pultz` is served and accepted as.
pub const MIME: &str = "application/vnd.pult.show+zip";

/// The largest show this console will unpack.
///
/// A gigabyte, unpacked. A rig's worth of GDTF archives and meshes is tens to
/// hundreds of megabytes, so this is well clear of anything real and still refuses a
/// zip bomb before it fills somebody's disk. The compressed body has its own,
/// smaller, limit at the route.
pub const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;

/// Zip a show up.
///
/// `pool` is the open show's read pool, so the copy is taken through SQLite rather
/// than off the disk — see the module note.
pub async fn export(bundle: &Bundle, pool: &SqlitePool, versions: bool) -> Result<Vec<u8>> {
    // A name nothing else will pick, inside the bundle, so the copy is on the same
    // volume as the show and the write cannot fail for want of room somewhere else.
    let staging = bundle.path().join(format!(".export-{}.db", uuid::Uuid::new_v4()));
    let _ = std::fs::remove_file(&staging);
    sqlx::query(&format!("VACUUM INTO '{}'", staging.display()))
        .execute(pool)
        .await
        .with_context(|| "copying the show for export")?;

    let written = zip_it(bundle, &staging, versions);
    // Whatever happened, the staging copy does not stay behind: it is a whole second
    // copy of the show, and one left in the bundle would be exported by the next one.
    let _ = std::fs::remove_file(&staging);
    let bytes = written?;
    info!("[shows] exported {} ({} bytes)", bundle.path().display(), bytes.len());
    Ok(bytes)
}

fn zip_it(bundle: &Bundle, db: &Path, versions: bool) -> Result<Vec<u8>> {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        // Deflate: a SQLite page file compresses well, and a mesh or a gobo image
        // barely at all. Storing everything would double what travels.
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("bundle.toml", options)?;
        zip.write_all(&std::fs::read(bundle.path().join("bundle.toml"))?)?;

        zip.start_file("show.db", options)?;
        zip.write_all(&std::fs::read(db)?)?;

        add_dir(&mut zip, options, &bundle.assets_dir(), "assets")?;
        if versions {
            add_dir(&mut zip, options, &bundle.versions_dir(), "versions")?;
        }
        zip.finish()?;
    }
    Ok(buffer.into_inner())
}

fn add_dir<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
    from: &Path,
    under: &str,
) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(from) else { return Ok(()) };
    // Sorted, so two exports of one show are the same bytes: a reader diffing them
    // should see what changed rather than what the filesystem felt like listing first.
    let mut names: Vec<std::ffi::OsString> = entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name())
        .collect();
    names.sort();
    for name in names {
        let Some(name) = name.to_str() else { continue };
        zip.start_file(format!("{under}/{name}"), options)?;
        zip.write_all(&std::fs::read(from.join(name))?)?;
    }
    Ok(())
}

/// Unpack a `.pultz` into `dir`, under whatever it calls itself.
///
/// The name comes from the manifest inside, and a name already taken gets a number
/// rather than an overwrite — the same rule *Save as…* follows, and for the same
/// reason: two shows honestly called *Rehearsal* is ordinary, and either alternative
/// loses somebody's work or makes them think of a different word.
pub fn import(zipped: &[u8], dir: &Path) -> Result<Bundle> {
    let mut archive = ZipArchive::new(Cursor::new(zipped))
        .with_context(|| "this does not look like a .pultz")?;

    // Read out of the archive before anything is written, so a file that is not a
    // show leaves no half-made folder behind.
    let manifest = read_entry(&mut archive, "bundle.toml")
        .ok_or_else(|| anyhow::anyhow!("there is no bundle.toml in this file"))?;
    let manifest: bundle::Manifest = toml::from_str(std::str::from_utf8(&manifest)?)
        .with_context(|| "reading the bundle.toml inside this file")?;
    if manifest.format != bundle::FORMAT {
        bail!(
            "this is a format {} show and this console reads {}",
            manifest.format,
            bundle::FORMAT
        );
    }
    let unpacked: u64 = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|entry| entry.size()))
        .sum();
    if unpacked > MAX_UNPACKED {
        bail!("this show unpacks to {unpacked} bytes, which is more than this console will take");
    }

    let into = bundle::free_path_in(dir, &manifest.name);
    let made = Bundle::create(&into, &manifest.name)?;

    let mut unpack = || -> Result<()> {
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let Some(name) = entry.enclosed_name() else {
                // `enclosed_name` is what refuses `../`: a zip is somebody else's
                // file, and an entry naming a path outside the folder is the oldest
                // trick there is.
                warn!("[shows] skipping an entry that names a path outside the show");
                continue;
            };
            if !allowed(&name) {
                warn!("[shows] skipping {} — not part of a show", name.display());
                continue;
            }
            let to = into.join(&name);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            std::fs::write(&to, bytes)?;
        }
        Ok(())
    };
    if let Err(e) = unpack() {
        // Half a show is worse than none: an operator would open it and find a rig
        // with no drawings in it.
        let _ = std::fs::remove_dir_all(&into);
        return Err(e);
    }
    info!("[shows] imported {}", into.display());
    Ok(made)
}

/// The parts of a bundle, and nothing else.
///
/// A `.pultz` is a file somebody sent, so what comes out of it is what this console
/// put in — not a `-wal` from a show that was open when it was written, and not
/// whatever else somebody added to the zip.
fn allowed(name: &Path) -> bool {
    let mut parts = name.components();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(one), None, None) => {
            matches!(one.as_os_str().to_str(), Some("bundle.toml") | Some("show.db"))
        }
        (Some(dir), Some(_), None) => {
            matches!(dir.as_os_str().to_str(), Some("assets") | Some("versions"))
        }
        _ => false,
    }
}

fn read_entry<R: Read + std::io::Seek>(archive: &mut ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// What a `.pultz` should be called when it is handed to a browser.
pub fn filename_for(bundle: &Bundle) -> String {
    let stem = bundle
        .path()
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("show");
    format!("{stem}.{}", bundle::TRAVEL_EXTENSION)
}

#[cfg(test)]
mod tests;
