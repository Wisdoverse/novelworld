use crate::domain::ports::{DocumentExtractionError, DocumentTextExtractor};
use encoding_rs::GBK;
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use zip::ZipArchive;

const MAX_TEXT_UPLOAD_SIZE: usize = 10 * 1024 * 1024;
const MAX_BINARY_UPLOAD_SIZE: usize = 20 * 1024 * 1024;
const MAX_EXTRACTED_TEXT_SIZE: usize = 20 * 1024 * 1024;
const MAX_EPUB_ENTRIES: usize = 10_000;
const MAX_CONTAINER_SIZE: usize = 256 * 1024;
const MAX_PACKAGE_SIZE: usize = 2 * 1024 * 1024;
const MAX_CHAPTER_SIZE: usize = 5 * 1024 * 1024;

pub struct EbookTextExtractor;

#[derive(Clone, Copy)]
enum DocumentFormat {
    Text,
    Epub,
    Pdf,
}

impl DocumentFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Text => "TXT",
            Self::Epub => "EPUB",
            Self::Pdf => "PDF",
        }
    }

    fn upload_limit(self) -> usize {
        match self {
            Self::Text => MAX_TEXT_UPLOAD_SIZE,
            Self::Epub | Self::Pdf => MAX_BINARY_UPLOAD_SIZE,
        }
    }
}

impl DocumentTextExtractor for EbookTextExtractor {
    fn extract_text(
        &self,
        file_name: Option<&str>,
        content_type: Option<&str>,
        data: &[u8],
    ) -> Result<String, DocumentExtractionError> {
        let format = detect_format(file_name, content_type, data)?;
        if data.len() > format.upload_limit() {
            return Err(DocumentExtractionError::UploadTooLarge {
                format: format.label(),
                max_bytes: format.upload_limit(),
            });
        }

        let text = match format {
            DocumentFormat::Text => decode_text(data)?,
            DocumentFormat::Epub => extract_epub_text(data)?,
            DocumentFormat::Pdf => pdf_extract::extract_text_from_mem(data)
                .map_err(|error| DocumentExtractionError::InvalidPdf(error.to_string()))?,
        };
        let text = normalize_text(&text);
        if text.is_empty() {
            return Err(DocumentExtractionError::EmptyDocument);
        }
        if text.len() > MAX_EXTRACTED_TEXT_SIZE {
            return Err(DocumentExtractionError::ExtractedTextTooLarge {
                max_bytes: MAX_EXTRACTED_TEXT_SIZE,
            });
        }
        Ok(text)
    }
}

fn detect_format(
    file_name: Option<&str>,
    content_type: Option<&str>,
    data: &[u8],
) -> Result<DocumentFormat, DocumentExtractionError> {
    if let Some(extension) = file_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
    {
        return match extension.as_str() {
            "txt" => Ok(DocumentFormat::Text),
            "epub" => Ok(DocumentFormat::Epub),
            "pdf" => Ok(DocumentFormat::Pdf),
            _ => Err(DocumentExtractionError::UnsupportedType),
        };
    }

    let mime = content_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match mime.as_str() {
        "text/plain" => Ok(DocumentFormat::Text),
        "application/epub+zip" => Ok(DocumentFormat::Epub),
        "application/pdf" => Ok(DocumentFormat::Pdf),
        "application/octet-stream" | "" if data.starts_with(b"PK\x03\x04") => {
            Ok(DocumentFormat::Epub)
        }
        "application/octet-stream" | "" if data.starts_with(b"%PDF-") => Ok(DocumentFormat::Pdf),
        _ => Err(DocumentExtractionError::UnsupportedType),
    }
}

fn decode_text(data: &[u8]) -> Result<String, DocumentExtractionError> {
    if let Some(bytes) = data.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(bytes.to_vec())
            .map_err(|_| DocumentExtractionError::InvalidTextEncoding);
    }
    if let Some(bytes) = data.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(bytes, true);
    }
    if let Some(bytes) = data.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(bytes, false);
    }
    if let Ok(text) = std::str::from_utf8(data) {
        return Ok(text.to_owned());
    }
    GBK.decode_without_bom_handling_and_without_replacement(data)
        .map(|text| text.into_owned())
        .ok_or(DocumentExtractionError::InvalidTextEncoding)
}

fn decode_utf16(data: &[u8], little_endian: bool) -> Result<String, DocumentExtractionError> {
    if !data.len().is_multiple_of(2) {
        return Err(DocumentExtractionError::InvalidTextEncoding);
    }
    let units = data.chunks_exact(2).map(|chunk| {
        let bytes = [chunk[0], chunk[1]];
        if little_endian {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    });
    std::char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| DocumentExtractionError::InvalidTextEncoding)
}

#[derive(Deserialize)]
struct ContainerDocument {
    rootfiles: RootFiles,
}

#[derive(Deserialize)]
struct RootFiles {
    #[serde(rename = "rootfile", default)]
    rootfiles: Vec<RootFile>,
}

#[derive(Deserialize)]
struct RootFile {
    #[serde(rename = "@full-path")]
    full_path: String,
}

#[derive(Deserialize)]
struct PackageDocument {
    manifest: Manifest,
    spine: Spine,
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "item", default)]
    items: Vec<ManifestItem>,
}

#[derive(Deserialize)]
struct ManifestItem {
    #[serde(rename = "@id")]
    id: String,
    #[serde(rename = "@href")]
    href: String,
    #[serde(rename = "@media-type", default)]
    media_type: String,
}

#[derive(Deserialize)]
struct Spine {
    #[serde(rename = "itemref", default)]
    items: Vec<SpineItem>,
}

#[derive(Deserialize)]
struct SpineItem {
    #[serde(rename = "@idref")]
    idref: String,
    #[serde(rename = "@linear", default)]
    linear: Option<String>,
}

fn extract_epub_text(data: &[u8]) -> Result<String, DocumentExtractionError> {
    let mut archive = ZipArchive::new(Cursor::new(data))
        .map_err(|error| invalid_epub(format!("cannot open archive: {error}")))?;
    if archive.len() > MAX_EPUB_ENTRIES {
        return Err(invalid_epub("archive contains too many entries"));
    }

    let mime = read_entry(&mut archive, "mimetype", 128)?;
    if mime.trim_ascii() != b"application/epub+zip" {
        return Err(invalid_epub("missing or invalid mimetype entry"));
    }

    let container = read_entry(&mut archive, "META-INF/container.xml", MAX_CONTAINER_SIZE)?;
    let container =
        std::str::from_utf8(&container).map_err(|_| invalid_epub("container.xml is not UTF-8"))?;
    let container: ContainerDocument = quick_xml::de::from_str(container)
        .map_err(|error| invalid_epub(format!("invalid container.xml: {error}")))?;
    let package_path = container
        .rootfiles
        .rootfiles
        .first()
        .map(|root| root.full_path.as_str())
        .ok_or_else(|| invalid_epub("container.xml has no rootfile"))?;
    let package_path = safe_archive_path(None, package_path)?;

    let package = read_entry(&mut archive, &package_path, MAX_PACKAGE_SIZE)?;
    let package =
        std::str::from_utf8(&package).map_err(|_| invalid_epub("package document is not UTF-8"))?;
    let package: PackageDocument = quick_xml::de::from_str(package)
        .map_err(|error| invalid_epub(format!("invalid package document: {error}")))?;

    let manifest: HashMap<_, _> = package
        .manifest
        .items
        .into_iter()
        .filter(|item| item.media_type == "application/xhtml+xml" || item.media_type == "text/html")
        .map(|item| (item.id, item.href))
        .collect();
    let mut result = String::new();
    for item in package.spine.items {
        if item.linear.as_deref() == Some("no") {
            continue;
        }
        let Some(href) = manifest.get(&item.idref) else {
            continue;
        };
        let chapter_path = safe_archive_path(Some(&package_path), href)?;
        let chapter = read_entry(&mut archive, &chapter_path, MAX_CHAPTER_SIZE)?;
        let chapter_text = html2text::from_read(chapter.as_slice(), 120)
            .map_err(|error| invalid_epub(format!("invalid XHTML in {chapter_path}: {error}")))?;
        if !chapter_text.trim().is_empty() {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str(chapter_text.trim());
            if result.len() > MAX_EXTRACTED_TEXT_SIZE {
                return Err(DocumentExtractionError::ExtractedTextTooLarge {
                    max_bytes: MAX_EXTRACTED_TEXT_SIZE,
                });
            }
        }
    }
    Ok(result)
}

fn read_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    limit: usize,
) -> Result<Vec<u8>, DocumentExtractionError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|_| invalid_epub(format!("missing archive entry: {name}")))?;
    if entry.size() > limit as u64 {
        return Err(invalid_epub(format!("archive entry is too large: {name}")));
    }
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry
        .by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|error| invalid_epub(format!("cannot read {name}: {error}")))?;
    if data.len() > limit {
        return Err(invalid_epub(format!("archive entry is too large: {name}")));
    }
    Ok(data)
}

fn safe_archive_path(
    package_path: Option<&str>,
    href: &str,
) -> Result<String, DocumentExtractionError> {
    let decoded = percent_decode_str(href.split(['#', '?']).next().unwrap_or_default())
        .decode_utf8()
        .map_err(|_| invalid_epub("resource path is not UTF-8"))?;
    let decoded = decoded.replace('\\', "/");
    let mut path = package_path
        .and_then(|path| Path::new(path).parent())
        .map(Path::to_path_buf)
        .unwrap_or_default();
    for component in Path::new(&decoded).components() {
        match component {
            Component::Normal(value) => path.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !path.pop() {
                    return Err(invalid_epub("resource path escapes the archive root"));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(invalid_epub("resource path must be relative"));
            }
        }
    }
    path_to_archive_name(path)
}

fn path_to_archive_name(path: PathBuf) -> Result<String, DocumentExtractionError> {
    let parts = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid_epub("resource path is not UTF-8")),
            _ => Err(invalid_epub("resource path is invalid")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.is_empty() {
        return Err(invalid_epub("resource path is empty"));
    }
    Ok(parts.join("/"))
}

fn normalize_text(text: &str) -> String {
    let text = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\0', "");
    let mut normalized = String::with_capacity(text.len());
    let mut blank_lines = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_lines += 1;
            if blank_lines > 1 {
                continue;
            }
        } else {
            blank_lines = 0;
        }
        if !normalized.is_empty() {
            normalized.push('\n');
        }
        normalized.push_str(line);
    }
    normalized.trim().to_owned()
}

fn invalid_epub(message: impl Into<String>) -> DocumentExtractionError {
    DocumentExtractionError::InvalidEpub(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn sample_epub() -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("mimetype", stored).unwrap();
        writer.write_all(b"application/epub+zip").unwrap();

        let options = SimpleFileOptions::default();
        writer
            .start_file("META-INF/container.xml", options)
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?>
                <container xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
                  <rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles>
                </container>"#,
            )
            .unwrap();
        writer.start_file("OPS/book.opf", options).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?>
                <package xmlns="http://www.idpf.org/2007/opf">
                  <manifest>
                    <item id="two" href="chapters/two.xhtml" media-type="application/xhtml+xml"/>
                    <item id="one" href="chapters/one.xhtml" media-type="application/xhtml+xml"/>
                  </manifest>
                  <spine><itemref idref="one"/><itemref idref="two"/></spine>
                </package>"#,
            )
            .unwrap();
        writer
            .start_file("OPS/chapters/one.xhtml", options)
            .unwrap();
        writer
            .write_all("<html><body><h1>第一章</h1><p>山雨欲来。</p></body></html>".as_bytes())
            .unwrap();
        writer
            .start_file("OPS/chapters/two.xhtml", options)
            .unwrap();
        writer
            .write_all(b"<html><body><h1>Chapter Two</h1><p>The journey begins.</p></body></html>")
            .unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn extracts_epub_in_spine_order() {
        let text = EbookTextExtractor
            .extract_text(
                Some("story.epub"),
                Some("application/octet-stream"),
                &sample_epub(),
            )
            .unwrap();
        assert!(text.find("第一章").unwrap() < text.find("Chapter Two").unwrap());
        assert!(text.contains("山雨欲来"));
        assert!(text.contains("The journey begins"));
    }

    #[test]
    fn decodes_utf8_utf16_and_gbk_text() {
        assert_eq!(
            decode_text("第一章\n正文".as_bytes()).unwrap(),
            "第一章\n正文"
        );
        let utf16 = [0xFF, 0xFE, 0x2C, 0x7B, 0x00, 0x4E, 0x87, 0x65];
        assert_eq!(decode_text(&utf16).unwrap(), "第一文");
        let (gbk, _, _) = GBK.encode("第一章");
        assert_eq!(decode_text(&gbk).unwrap(), "第一章");
    }

    #[test]
    fn rejects_unsupported_extensions_and_archive_traversal() {
        assert!(matches!(
            detect_format(Some("story.docx"), None, b"PK\x03\x04"),
            Err(DocumentExtractionError::UnsupportedType)
        ));
        assert!(safe_archive_path(None, "../outside.xhtml").is_err());
    }
}
