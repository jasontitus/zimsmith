//! Stream WARC records into the existing offline import pipeline.
use crate::{imports::Payload, web, Record};
use anyhow::{Context, Result};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    sync::{atomic::Ordering, Arc},
};
use warc::{RecordType, WarcHeader, WarcReader};
const HEADER_LIMIT: usize = 64 * 1024;
const MAX_RECORDS: usize = 200_000;
pub(crate) fn recognized(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.ends_with(".warc") || path.ends_with(".warc.gz")
}

// The upstream streaming parser uses read_until for headers. Bound those
// allocations as well as the total expanded stream and check cancellation.
struct Limited<R> {
    inner: R,
    left: u64,
    control: Arc<Record>,
}
impl<R: BufRead> Read for Limited<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let buf = self.fill_buf()?;
        let n = buf.len().min(out.len());
        out[..n].copy_from_slice(&buf[..n]);
        self.consume(n);
        Ok(n)
    }
}
impl<R: BufRead> BufRead for Limited<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.control.import_cancel.load(Ordering::SeqCst) {
            return Err(io::Error::other("Import interrupted"));
        }
        let buf = self.inner.fill_buf()?;
        if self.left == 0 && !buf.is_empty() {
            return Err(io::Error::other("Expanded WARC exceeds the import budget"));
        }
        Ok(&buf[..buf.len().min(self.left as usize)])
    }
    fn consume(&mut self, n: usize) {
        self.left -= n as u64;
        self.inner.consume(n);
    }
    fn read_until(&mut self, delimiter: u8, out: &mut Vec<u8>) -> io::Result<usize> {
        let start = out.len();
        loop {
            let buf = self.fill_buf()?;
            if buf.is_empty() {
                if !out.is_empty() {
                    return Err(io::Error::other("Truncated WARC header"));
                }
                return Ok(0);
            }
            let end = buf.iter().position(|b| *b == delimiter).map(|n| n + 1);
            let n = end.unwrap_or(buf.len());
            if out.len() + n > HEADER_LIMIT {
                return Err(io::Error::other("WARC header exceeds 64 KB"));
            }
            out.extend_from_slice(&buf[..n]);
            self.consume(n);
            if end.is_some() {
                return Ok(out.len() - start);
            }
        }
    }
}

// A strict, bounded HTTP chunk decoder. Replay-oriented decoders can accept
// truncated chunks; an importer must not label their partial payload complete.
struct Chunked<R> {
    inner: R,
    left: u64,
    separator: bool,
    done: bool,
}
fn line<R: BufRead>(input: &mut R) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let bytes = input.fill_buf()?;
        if bytes.is_empty() {
            return Err(io::Error::other("Truncated HTTP chunk header"));
        }
        let end = bytes.iter().position(|b| *b == b'\n').map(|n| n + 1);
        let n = end.unwrap_or(bytes.len());
        if out.len() + n > HEADER_LIMIT {
            return Err(io::Error::other("HTTP chunk header exceeds 64 KB"));
        }
        out.extend_from_slice(&bytes[..n]);
        input.consume(n);
        if end.is_some() {
            if !out.ends_with(b"\r\n") {
                return Err(io::Error::other("Invalid HTTP chunk header"));
            }
            return Ok(out);
        }
    }
}
impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.done {
            return Ok(0);
        }
        if self.separator {
            let mut bytes = [0; 2];
            self.inner.read_exact(&mut bytes)?;
            if bytes != *b"\r\n" {
                return Err(io::Error::other("Invalid HTTP chunk terminator"));
            }
            self.separator = false;
        }
        if self.left == 0 {
            let header = line(&mut self.inner)?;
            let digits = header[..header.len() - 2]
                .split(|b| *b == b';')
                .next()
                .unwrap();
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_hexdigit) {
                return Err(io::Error::other("Invalid HTTP chunk size"));
            }
            self.left = u64::from_str_radix(std::str::from_utf8(digits).unwrap(), 16)
                .map_err(|_| io::Error::other("Invalid HTTP chunk size"))?;
            if self.left == 0 {
                let mut total = 0;
                loop {
                    let trailer = line(&mut self.inner)?;
                    total += trailer.len();
                    if total > HEADER_LIMIT {
                        return Err(io::Error::other("HTTP trailers exceed 64 KB"));
                    }
                    if trailer == b"\r\n" {
                        break;
                    }
                    if !trailer.contains(&b':') {
                        return Err(io::Error::other("Invalid HTTP trailer"));
                    }
                }
                if self.inner.read(&mut [0])? != 0 {
                    return Err(io::Error::other("Extra data after HTTP chunked body"));
                }
                self.done = true;
                return Ok(0);
            }
        }
        let len = (out.len() as u64).min(self.left) as usize;
        let n = self.inner.read(&mut out[..len])?;
        if n == 0 {
            return Err(io::Error::other("Truncated HTTP chunk"));
        }
        self.left -= n as u64;
        self.separator = self.left == 0;
        Ok(n)
    }
}

pub(crate) struct Expansion {
    pub payloads: Vec<Payload>,
    pub budget_used: u64,
}

pub(crate) fn expand(
    file: &Path,
    gzip: bool,
    output: &Path,
    cap: u64,
    file_cap: u64,
    control: Arc<Record>,
) -> Result<Expansion> {
    let input = std::fs::File::open(file)?;
    let input: Box<dyn Read> = if gzip {
        Box::new(flate2::read::MultiGzDecoder::new(input))
    } else {
        Box::new(input)
    };
    let mut limited = Limited {
        inner: BufReader::new(input),
        left: cap,
        control: control.clone(),
    };
    let mut reader = WarcReader::new(&mut limited);
    let mut records = reader.stream_records();
    let mut payloads = BTreeMap::new();
    let mut total = 0u64;
    let mut count = 0usize;
    let mut skipped = 0usize;
    let mut invalid = 0usize;
    let mut first_invalid_header = None;
    std::fs::create_dir_all(output)?;
    while let Some(record) = records.next_item() {
        let mut record = record.context("Invalid WARC record")?;
        count += 1;
        anyhow::ensure!(
            count <= MAX_RECORDS,
            "WARC exceeds the {MAX_RECORDS}-record limit"
        );
        if let Ok(mut job) = control.job.try_lock() {
            job.progress.label = format!("Reading WARC record {count}");
            job.progress.completed = count as u64;
            job.progress.unit = "records".into();
            job.progress.updated_at = crate::now();
        }
        let kind = record.warc_type().clone();
        if !matches!(kind, RecordType::Response | RecordType::Resource) {
            if matches!(
                kind,
                RecordType::Revisit | RecordType::Continuation | RecordType::Conversion
            ) {
                skipped += 1;
            }
            continue;
        }
        if record.truncated_type().is_some()
            || record.header(WarcHeader::Truncated).is_some()
            || record.header(WarcHeader::SegmentNumber).is_some()
        {
            skipped += 1;
            continue;
        }
        let Some(mut url) = record
            .header(WarcHeader::TargetURI)
            .and_then(|target| web::http_url(target.trim_matches(['<', '>'])).ok())
        else {
            invalid += 1;
            continue;
        };
        url.set_fragment(None);
        let url = url.to_string();
        let declared = record.content_length();
        anyhow::ensure!(
            declared <= file_cap + HEADER_LIMIT as u64,
            "WARC record exceeds the per-file limit"
        );
        let initial_type = if kind == RecordType::Response {
            WarcHeader::IdentifiedPayloadType
        } else {
            WarcHeader::ContentType
        };
        let mut mime = record
            .header(initial_type)
            .as_deref()
            .and_then(web::normalized_mime);
        let mut encoding = String::new();
        let mut chunked = false;
        if kind == RecordType::Response {
            let mut bytes = Vec::new();
            let mut byte = [0];
            while !bytes.ends_with(b"\r\n\r\n") {
                anyhow::ensure!(bytes.len() < HEADER_LIMIT, "HTTP header exceeds 64 KB");
                record
                    .read_exact(&mut byte)
                    .context("Truncated HTTP response header")?;
                bytes.push(byte[0]);
            }
            let mut headers = [httparse::EMPTY_HEADER; 128];
            let mut response = httparse::Response::new(&mut headers);
            // Captures can contain legacy folded headers or malformed server
            // headers. The enclosing WARC length lets us skip only that record.
            let parsed = httparse::ParserConfig::default()
                .allow_obsolete_multiline_headers_in_responses(true)
                .parse_response(&mut response, &bytes);
            match parsed {
                Ok(status) if status.is_complete() => {}
                result => {
                    invalid += 1;
                    first_invalid_header.get_or_insert_with(|| {
                        let reason = result
                            .err()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "incomplete response header".into());
                        format!("record {count}: {reason}")
                    });
                    continue;
                }
            }
            let status = response.code.context("Missing HTTP status")?;
            if !(200..300).contains(&status) || status == 206 {
                skipped += 1;
                continue;
            }
            let mut http_length = None;
            for h in response.headers {
                if ![
                    "content-length",
                    "content-type",
                    "content-encoding",
                    "transfer-encoding",
                ]
                .iter()
                .any(|name| h.name.eq_ignore_ascii_case(name))
                {
                    continue;
                }
                let value = std::str::from_utf8(h.value)?.trim();
                if h.name.eq_ignore_ascii_case("content-length") {
                    anyhow::ensure!(http_length.is_none(), "Duplicate HTTP content length");
                    http_length = Some(
                        value
                            .parse::<u64>()
                            .context("Invalid HTTP content length")?,
                    );
                }
                if h.name.eq_ignore_ascii_case("content-type") {
                    if let Some(content_type) = web::normalized_mime(value) {
                        mime = Some(content_type);
                    }
                }
                if h.name.eq_ignore_ascii_case("content-encoding") {
                    encoding = value.to_ascii_lowercase();
                }
                if h.name.eq_ignore_ascii_case("transfer-encoding") {
                    anyhow::ensure!(
                        value.eq_ignore_ascii_case("chunked")
                            || value.eq_ignore_ascii_case("identity"),
                        "Unsupported WARC HTTP transfer encoding"
                    );
                    chunked = value.eq_ignore_ascii_case("chunked");
                }
            }
            if !chunked {
                if let Some(expected) = http_length {
                    anyhow::ensure!(
                        expected == declared.saturating_sub(bytes.len() as u64),
                        "HTTP content length does not match the WARC record"
                    );
                }
            }
        }
        let mime = mime.unwrap_or_else(|| "application/octet-stream".into());
        anyhow::ensure!(
            payloads.contains_key(&url) || payloads.len() < crate::imports::MAX_FILES,
            "WARC exceeds the 50,000 unique capture limit"
        );
        let key = web::key(&url);
        let path = format!("warc-{key}");
        let file = output.join(&path);
        let mut raw: Box<dyn Read + '_> = if chunked {
            Box::new(Chunked {
                inner: BufReader::new(&mut record),
                left: 0,
                separator: false,
                done: false,
            })
        } else {
            Box::new(&mut record)
        };
        let mut body: Box<dyn Read + '_> = match encoding.as_str() {
            "" | "identity" => Box::new(&mut raw),
            "gzip" | "x-gzip" => Box::new(flate2::read::MultiGzDecoder::new(&mut raw)),
            "deflate" => Box::new(flate2::read::ZlibDecoder::new(&mut raw)),
            _ => anyhow::bail!("Unsupported WARC HTTP content encoding: {encoding}"),
        };
        let mut dest = std::fs::File::create(&file)?;
        let mut size = 0u64;
        let mut buffer = [0; 64 * 1024];
        loop {
            anyhow::ensure!(
                !control.import_cancel.load(Ordering::SeqCst),
                "Import interrupted"
            );
            let n = body.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            total += n as u64;
            anyhow::ensure!(
                size <= file_cap && total <= cap,
                "Decoded WARC exceeds its file or total import budget"
            );
            dest.write_all(&buffer[..n])?;
        }
        drop(body);
        anyhow::ensure!(
            raw.read(&mut [0])? == 0,
            "Extra data after decoded HTTP body"
        );
        // Staging is rebuilt after interruption. Avoid a disk sync for every record.
        drop(dest);
        payloads.insert(
            url.clone(),
            Payload {
                path,
                file,
                size,
                url: Some(url),
                mime: Some(mime),
            },
        );
    }
    if let Ok(mut job) = control.job.try_lock() {
        if skipped > 0 {
            job.log("warning", format!("WARC import omitted {skipped} redirect, error, partial, revisit, conversion or segmented records. Only complete captured response/resource bodies are imported."));
        }
        if invalid > 0 {
            let detail = first_invalid_header
                .map(|reason| format!(" First invalid header: {reason}."))
                .unwrap_or_default();
            job.log("warning", format!("WARC import omitted {invalid} records with invalid captured HTTP headers or target URLs.{detail}"));
        }
    }
    drop(records);
    drop(reader);
    Ok(Expansion {
        payloads: payloads.into_values().collect(),
        budget_used: (cap - limited.left).max(total),
    })
}
