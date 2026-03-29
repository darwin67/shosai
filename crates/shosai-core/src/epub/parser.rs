//! EPUB parsing: ZIP extraction, container.xml, OPF, and content loading.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use zip::ZipArchive;

use super::types::*;
use crate::document::DocumentMetadata;

/// A parsed EPUB document.
#[derive(Debug)]
pub struct EpubDoc {
    pub content: EpubContent,
}

impl EpubDoc {
    /// Open an EPUB file from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data =
            std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_bytes(data)
    }

    /// Open an EPUB from raw bytes.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let cursor = Cursor::new(data);
        let mut archive = ZipArchive::new(cursor).context("failed to open EPUB as ZIP archive")?;

        // 1. Parse container.xml to find the OPF path.
        let opf_path = parse_container(&mut archive)?;

        // The OPF directory is used as a base for resolving relative paths.
        let opf_dir = PathBuf::from(&opf_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        // 2. Parse the OPF file.
        let opf_xml = read_archive_entry(&mut archive, &opf_path)
            .with_context(|| format!("failed to read OPF file: {opf_path}"))?;
        let (metadata, manifest, spine_ids) = parse_opf(&opf_xml, &opf_dir)?;

        // 3. Try to parse the TOC (NCX or nav document).
        let toc = parse_toc(&mut archive, &manifest, &opf_dir);

        // 4. Load chapters (spine items) in reading order.
        let chapters = load_chapters(&mut archive, &spine_ids, &manifest, &toc)?;

        // 5. Load resources (images, CSS, fonts).
        let resources = load_resources(&mut archive, &manifest)?;

        // 6. Parse CSS stylesheets into a class → style map.
        let css_sources: Vec<(&str, String)> = manifest
            .values()
            .filter(|item| item.media_type == "text/css")
            .filter_map(|item| {
                resources
                    .get(&item.href)
                    .and_then(|data| String::from_utf8(data.clone()).ok())
                    .map(|css| (item.href.as_str(), css))
            })
            .collect();

        let styles = super::style::parse_epub_styles(
            css_sources.iter().map(|(path, css)| (*path, css.as_str())),
        );

        Ok(Self {
            content: EpubContent {
                metadata,
                chapters,
                toc,
                manifest,
                resources,
                styles,
            },
        })
    }

    /// Number of chapters.
    pub fn chapter_count(&self) -> usize {
        self.content.chapters.len()
    }

    /// Get a chapter by index.
    pub fn chapter(&self, index: usize) -> Option<&Chapter> {
        self.content.chapters.get(index)
    }

    /// Get document metadata in the common format.
    pub fn metadata(&self) -> DocumentMetadata {
        DocumentMetadata {
            title: self.content.metadata.title.clone(),
            author: self.content.metadata.author.clone(),
            subject: self.content.metadata.description.clone(),
            creator: None,
        }
    }

    /// Get a resource (image, CSS, etc.) by its archive path.
    pub fn resource(&self, path: &str) -> Option<&[u8]> {
        self.content.resources.get(path).map(|v| v.as_slice())
    }

    /// Get the table of contents.
    pub fn toc(&self) -> &[TocEntry] {
        &self.content.toc
    }
}

// ---------------------------------------------------------------------------
// Internal parsing functions
// ---------------------------------------------------------------------------

/// Read a file from the ZIP archive as a UTF-8 string.
fn read_archive_entry(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Result<String> {
    let mut file = archive
        .by_name(name)
        .with_context(|| format!("entry not found in archive: {name}"))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .with_context(|| format!("failed to read archive entry: {name}"))?;
    Ok(buf)
}

/// Read a file from the ZIP archive as raw bytes.
fn read_archive_bytes(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Result<Vec<u8>> {
    let mut file = archive
        .by_name(name)
        .with_context(|| format!("entry not found in archive: {name}"))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .with_context(|| format!("failed to read archive entry: {name}"))?;
    Ok(buf)
}

/// Parse META-INF/container.xml to find the OPF file path.
fn parse_container(archive: &mut ZipArchive<Cursor<Vec<u8>>>) -> Result<String> {
    let xml = read_archive_entry(archive, "META-INF/container.xml")
        .context("EPUB missing META-INF/container.xml")?;

    let doc = roxmltree::Document::parse(&xml).context("failed to parse container.xml")?;

    // Find <rootfile full-path="..."/>
    let rootfile = doc
        .descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .context("container.xml missing <rootfile> element")?;

    let full_path = rootfile
        .attribute("full-path")
        .context("rootfile missing full-path attribute")?;

    Ok(full_path.to_string())
}

/// Parse the OPF file, returning metadata, manifest items, and spine item IDs.
fn parse_opf(
    xml: &str,
    opf_dir: &str,
) -> Result<(EpubMetadata, HashMap<String, ManifestItem>, Vec<String>)> {
    let doc = roxmltree::Document::parse(xml).context("failed to parse OPF file")?;

    // -- Metadata --
    let mut metadata = EpubMetadata::default();

    for node in doc.descendants() {
        if node.is_element() {
            match node.tag_name().name() {
                "title" => {
                    if metadata.title.is_none() {
                        metadata.title = node.text().map(|s| s.trim().to_string());
                    }
                }
                "creator" => {
                    if metadata.author.is_none() {
                        metadata.author = node.text().map(|s| s.trim().to_string());
                    }
                }
                "language" => {
                    if metadata.language.is_none() {
                        metadata.language = node.text().map(|s| s.trim().to_string());
                    }
                }
                "publisher" => {
                    if metadata.publisher.is_none() {
                        metadata.publisher = node.text().map(|s| s.trim().to_string());
                    }
                }
                "description" => {
                    if metadata.description.is_none() {
                        metadata.description = node.text().map(|s| s.trim().to_string());
                    }
                }
                "meta" => {
                    // <meta name="cover" content="cover-image-id"/>
                    if node.attribute("name") == Some("cover") {
                        metadata.cover_image_id = node.attribute("content").map(|s| s.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    // -- Manifest --
    let mut manifest = HashMap::new();

    for node in doc.descendants() {
        if node.is_element()
            && node.tag_name().name() == "item"
            && let (Some(id), Some(href), Some(media_type)) = (
                node.attribute("id"),
                node.attribute("href"),
                node.attribute("media-type"),
            )
        {
            let full_href = resolve_path(opf_dir, href);
            manifest.insert(
                id.to_string(),
                ManifestItem {
                    id: id.to_string(),
                    href: full_href,
                    media_type: media_type.to_string(),
                },
            );
        }
    }

    // -- Spine --
    let mut spine_ids = Vec::new();

    // Find the <spine> element
    if let Some(spine_el) = doc.descendants().find(|n| n.tag_name().name() == "spine") {
        for child in spine_el.children() {
            if child.is_element()
                && child.tag_name().name() == "itemref"
                && let Some(idref) = child.attribute("idref")
            {
                spine_ids.push(idref.to_string());
            }
        }
    }

    Ok((metadata, manifest, spine_ids))
}

/// Try to parse the table of contents (NCX or EPUB3 nav).
fn parse_toc(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    manifest: &HashMap<String, ManifestItem>,
    opf_dir: &str,
) -> Vec<TocEntry> {
    // Try NCX first (EPUB 2).
    if let Some(ncx_item) = manifest
        .values()
        .find(|item| item.media_type == "application/x-dtbncx+xml")
        && let Ok(xml) = read_archive_entry(archive, &ncx_item.href)
        && let Ok(entries) = parse_ncx_toc(&xml, opf_dir)
    {
        return entries;
    }

    // Try EPUB 3 nav document.
    if let Some(nav_item) = manifest
        .values()
        .find(|item| item.media_type == "application/xhtml+xml" && item.id.contains("nav"))
        && let Ok(xml) = read_archive_entry(archive, &nav_item.href)
        && let Ok(entries) = parse_nav_toc(&xml, opf_dir)
    {
        return entries;
    }

    Vec::new()
}

/// Parse an NCX (EPUB 2) table of contents.
fn parse_ncx_toc(xml: &str, opf_dir: &str) -> Result<Vec<TocEntry>> {
    let doc = roxmltree::Document::parse(xml).context("failed to parse NCX")?;

    fn parse_navpoints(parent: roxmltree::Node, opf_dir: &str) -> Vec<TocEntry> {
        let mut entries = Vec::new();
        for child in parent.children() {
            if child.is_element() && child.tag_name().name() == "navPoint" {
                let title = child
                    .descendants()
                    .find(|n| n.tag_name().name() == "text")
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .trim()
                    .to_string();

                let href = child
                    .descendants()
                    .find(|n| n.tag_name().name() == "content")
                    .and_then(|n| n.attribute("src"))
                    .map(|s| resolve_path(opf_dir, s))
                    .unwrap_or_default();

                let children = parse_navpoints(child, opf_dir);

                entries.push(TocEntry {
                    title,
                    href,
                    children,
                });
            }
        }
        entries
    }

    // Find <navMap>
    let nav_map = doc
        .descendants()
        .find(|n| n.tag_name().name() == "navMap")
        .context("NCX missing <navMap>")?;

    Ok(parse_navpoints(nav_map, opf_dir))
}

/// Parse an EPUB 3 nav document table of contents.
fn parse_nav_toc(xml: &str, opf_dir: &str) -> Result<Vec<TocEntry>> {
    let doc = roxmltree::Document::parse(xml).context("failed to parse nav document")?;

    // Find <nav epub:type="toc"> or just <nav>
    let nav = doc
        .descendants()
        .find(|n| n.tag_name().name() == "nav")
        .context("nav document missing <nav> element")?;

    // Find the <ol> inside
    let ol = nav
        .descendants()
        .find(|n| n.tag_name().name() == "ol")
        .context("nav missing <ol>")?;

    Ok(parse_nav_ol(ol, opf_dir))
}

fn parse_nav_ol(ol: roxmltree::Node, opf_dir: &str) -> Vec<TocEntry> {
    let mut entries = Vec::new();
    for li in ol.children() {
        if !li.is_element() || li.tag_name().name() != "li" {
            continue;
        }

        let (title, href) = if let Some(a) = li
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "a")
        {
            let title = a.text().unwrap_or("").trim().to_string();
            let href = a
                .attribute("href")
                .map(|s| resolve_path(opf_dir, s))
                .unwrap_or_default();
            (title, href)
        } else {
            continue;
        };

        let children = li
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "ol")
            .map(|ol| parse_nav_ol(ol, opf_dir))
            .unwrap_or_default();

        entries.push(TocEntry {
            title,
            href,
            children,
        });
    }
    entries
}

/// Load chapters in spine order, assigning titles from the TOC where possible.
fn load_chapters(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    spine_ids: &[String],
    manifest: &HashMap<String, ManifestItem>,
    toc: &[TocEntry],
) -> Result<Vec<Chapter>> {
    // Build a map from (path without fragment) -> TOC title for quick lookup.
    let mut toc_titles: HashMap<String, String> = HashMap::new();
    fn collect_titles(entries: &[TocEntry], map: &mut HashMap<String, String>) {
        for entry in entries {
            let path = entry.href.split('#').next().unwrap_or("").to_string();
            if !path.is_empty() {
                map.entry(path).or_insert_with(|| entry.title.clone());
            }
            collect_titles(&entry.children, map);
        }
    }
    collect_titles(toc, &mut toc_titles);

    let mut chapters = Vec::new();

    for (index, id) in spine_ids.iter().enumerate() {
        let item = manifest
            .get(id)
            .with_context(|| format!("spine references unknown manifest id: {id}"))?;

        // Only load XHTML content documents.
        if item.media_type != "application/xhtml+xml" {
            continue;
        }

        let content = read_archive_entry(archive, &item.href)
            .with_context(|| format!("failed to read chapter: {}", item.href))?;

        let title = toc_titles.get(&item.href).cloned();

        chapters.push(Chapter {
            index,
            title,
            path: item.href.clone(),
            content,
        });
    }

    Ok(chapters)
}

/// Load non-XHTML resources (images, CSS, fonts) from the manifest.
fn load_resources(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    manifest: &HashMap<String, ManifestItem>,
) -> Result<HashMap<String, Vec<u8>>> {
    let mut resources = HashMap::new();

    for item in manifest.values() {
        // Skip XHTML documents (chapters) and NCX.
        if item.media_type == "application/xhtml+xml"
            || item.media_type == "application/x-dtbncx+xml"
        {
            continue;
        }

        // Best effort: skip resources we can't read (e.g. missing from archive).
        if let Ok(data) = read_archive_bytes(archive, &item.href) {
            resources.insert(item.href.clone(), data);
        }
    }

    Ok(resources)
}

/// Resolve a relative path against the OPF directory.
fn resolve_path(opf_dir: &str, href: &str) -> String {
    if opf_dir.is_empty() {
        href.to_string()
    } else {
        format!("{opf_dir}/{href}")
    }
}
