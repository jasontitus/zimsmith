//! Put playback metadata before media for readers using WebKit custom URL schemes.
//! Only accepts the progressive layout produced by our MP4 writer. Encoded media
//! and timing tables stay unchanged; only chunk byte offsets are adjusted.
use anyhow::{Context, Result};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_MOVIE_BYTES: u64 = 16 * 1024 * 1024;
const COPY_BYTES: usize = 256 * 1024;

fn active(stop: &AtomicBool) -> Result<()> {
    anyhow::ensure!(!stop.load(Ordering::Relaxed), "MP4 combining paused");
    Ok(())
}

pub(crate) fn fast_start(file: &mut File, stop: &AtomicBool) -> Result<()> {
    active(stop)?;
    let length = file.metadata()?.len();
    let mut boxes = Vec::new();
    let mut position = 0_u64;
    while position < length {
        anyhow::ensure!(
            boxes.len() < 4 && length - position >= 8,
            "Invalid generated MP4 layout"
        );
        file.seek(SeekFrom::Start(position))?;
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let mut size = u64::from(u32::from_be_bytes(header[..4].try_into()?));
        let kind: [u8; 4] = header[4..].try_into()?;
        let header_size = if size == 1 {
            file.read_exact(&mut header)?;
            size = u64::from_be_bytes(header);
            16
        } else {
            8
        };
        anyhow::ensure!(
            size >= header_size && size <= length - position,
            "Invalid generated MP4 box"
        );
        boxes.push((kind, position, size, header_size));
        position += size;
    }
    let kinds: Vec<_> = boxes.iter().map(|b| &b.0).collect();
    anyhow::ensure!(
        kinds == vec![b"ftyp", b"free", b"mdat", b"moov"]
            || kinds == vec![b"ftyp", b"mdat", b"moov"],
        "Unexpected generated MP4 layout"
    );
    let (_, insert, _, _) = boxes[1];
    let (_, movie_at, movie_size, movie_header) = *boxes.last().unwrap();
    anyhow::ensure!(
        movie_size <= MAX_MOVIE_BYTES && movie_header == 8,
        "MP4 playback metadata exceeds memory limit"
    );
    let (_, media_at, media_size, media_header) = boxes[boxes.len() - 2];
    let mut movie = vec![0; usize::try_from(movie_size)?];
    file.seek(SeekFrom::Start(movie_at))?;
    file.read_exact(&mut movie)?;
    adjust_offsets(
        &mut movie[8..],
        movie_size,
        media_at + media_header,
        media_at + media_size,
        0,
    )?;

    // Shift from the end so overlapping writes cannot overwrite unread media.
    // Work on the disposable partial file; cancellation never replaces a good output.
    let mut buffer = vec![0; COPY_BYTES];
    let mut end = movie_at;
    while end > insert {
        active(stop)?;
        let size = usize::try_from((end - insert).min(COPY_BYTES as u64))?;
        let start = end - size as u64;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..size])?;
        file.seek(SeekFrom::Start(start + movie_size))?;
        file.write_all(&buffer[..size])?;
        end = start;
    }
    active(stop)?;
    file.seek(SeekFrom::Start(insert))?;
    file.write_all(&movie)?;
    file.flush()?;
    anyhow::ensure!(
        file.metadata()?.len() == length,
        "MP4 layout changed file length"
    );
    Ok(())
}

fn adjust_offsets(
    data: &mut [u8],
    shift: u64,
    media_start: u64,
    media_end: u64,
    depth: u8,
) -> Result<()> {
    anyhow::ensure!(depth <= 5, "Invalid MP4 metadata nesting");
    let mut position = 0;
    while position < data.len() {
        let header = data
            .get(position..position + 8)
            .context("Truncated MP4 metadata box")?;
        let size = usize::try_from(u32::from_be_bytes(header[..4].try_into()?))?;
        let kind: [u8; 4] = header[4..].try_into()?;
        anyhow::ensure!(
            size >= 8 && size <= data.len() - position,
            "Invalid MP4 metadata size"
        );
        let body = &mut data[position + 8..position + size];
        match &kind {
            b"trak" | b"mdia" | b"minf" | b"stbl" => {
                adjust_offsets(body, shift, media_start, media_end, depth + 1)?
            }
            b"stco" | b"co64" => {
                anyhow::ensure!(
                    body.len() >= 8 && body[..4] == [0; 4],
                    "Invalid MP4 chunk-offset header"
                );
                let count = usize::try_from(u32::from_be_bytes(body[4..8].try_into()?))?;
                let width = if &kind == b"stco" { 4 } else { 8 };
                anyhow::ensure!(
                    count == (body.len() - 8) / width && (body.len() - 8) % width == 0,
                    "Invalid MP4 chunk-offset count"
                );
                for chunk in body[8..].chunks_exact_mut(width) {
                    let offset = if width == 4 {
                        u64::from(u32::from_be_bytes(chunk[..].try_into()?))
                    } else {
                        u64::from_be_bytes(chunk[..].try_into()?)
                    };
                    anyhow::ensure!(
                        (media_start..media_end).contains(&offset),
                        "MP4 chunk lies outside media data"
                    );
                    let offset = offset
                        .checked_add(shift)
                        .context("MP4 chunk offset overflow")?;
                    if width == 4 {
                        chunk.copy_from_slice(
                            &u32::try_from(offset)
                                .context("MP4 chunk requires 64-bit offsets")?
                                .to_be_bytes(),
                        );
                    } else {
                        chunk.copy_from_slice(&offset.to_be_bytes());
                    }
                }
            }
            _ => {}
        }
        position += size;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moves_overlapping_media_in_bounded_chunks_and_preserves_bytes() {
        let mut file = tempfile::tempfile().unwrap();
        let payload: Vec<_> = (0..COPY_BYTES * 3 + 17).map(|i| (i % 251) as u8).collect();
        let media_start = 24_u64;
        let movie = [
            32_u32.to_be_bytes().as_slice(),
            b"moov",
            &24_u32.to_be_bytes(),
            b"co64",
            &[0; 4],
            &1_u32.to_be_bytes(),
            &media_start.to_be_bytes(),
        ]
        .concat();
        file.write_all(&8_u32.to_be_bytes()).unwrap();
        file.write_all(b"ftyp").unwrap();
        file.write_all(&8_u32.to_be_bytes()).unwrap();
        file.write_all(b"free").unwrap();
        file.write_all(&((payload.len() + 8) as u32).to_be_bytes())
            .unwrap();
        file.write_all(b"mdat").unwrap();
        file.write_all(&payload).unwrap();
        file.write_all(&movie).unwrap();
        let stop = AtomicBool::new(true);
        let size = file.metadata().unwrap().len();
        assert!(fast_start(&mut file, &stop).is_err());
        assert_eq!(file.metadata().unwrap().len(), size);
        stop.store(false, Ordering::Relaxed);
        fast_start(&mut file, &stop).unwrap();
        file.seek(SeekFrom::Start(8)).unwrap();
        let mut actual = Vec::new();
        file.read_to_end(&mut actual).unwrap();
        assert_eq!(&actual[4..8], b"moov");
        assert_eq!(
            u64::from_be_bytes(actual[24..32].try_into().unwrap()),
            media_start + 32
        );
        assert_eq!(&actual[48..], payload.as_slice());
    }
    #[test]
    fn adjusts_large_chunk_offsets_without_truncation() {
        let offset = u64::from(u32::MAX) - 8;
        let mut table = [
            24_u32.to_be_bytes().as_slice(),
            b"co64",
            &[0; 4],
            &1_u32.to_be_bytes(),
            &offset.to_be_bytes(),
        ]
        .concat();
        adjust_offsets(&mut table, 1000, offset, offset + 100, 0).unwrap();
        assert_eq!(
            u64::from_be_bytes(table[16..24].try_into().unwrap()),
            offset + 1000
        );
        let mut narrow = [
            20_u32.to_be_bytes().as_slice(),
            b"stco",
            &[0; 4],
            &1_u32.to_be_bytes(),
            &(offset as u32).to_be_bytes(),
        ]
        .concat();
        assert!(adjust_offsets(&mut narrow, 1000, offset, offset + 100, 0).is_err());
    }
    #[test]
    fn rejects_malformed_or_out_of_range_tables() {
        for mut table in [
            vec![0; 8],
            [
                16_u32.to_be_bytes().as_slice(),
                b"co64",
                &[0; 4],
                &1_u32.to_be_bytes(),
            ]
            .concat(),
            [
                20_u32.to_be_bytes().as_slice(),
                b"stco",
                &[0; 4],
                &1_u32.to_be_bytes(),
                &500_u32.to_be_bytes(),
            ]
            .concat(),
        ] {
            assert!(adjust_offsets(&mut table, 100, 1000, 2000, 0).is_err());
        }
    }
}
