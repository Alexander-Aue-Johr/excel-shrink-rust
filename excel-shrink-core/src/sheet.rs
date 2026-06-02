use crate::error::{Result, ShrinkError};
use crate::models::{CleanSheetParameters, SheetStreamingState};
use bstr::ByteSlice;
use regex::bytes::{Captures, Regex};
use tracing::{debug, trace};

pub fn clean_sheet_data_streamwise(
    mut stream: impl std::io::Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    let mut output = Vec::new();
    let mut state = SheetStreamingState::SeekWorksheetNode;
    let mut eof_reached = false;
    let mut read_buf = vec![0u8; chunk_size];

    let row_clean_pattern = Regex::new(
        r#"(?s)<row\b(?:[^>]*\br="([^"]+?)")?(?:[^>]*\bhidden="(1)")?(?:[^>]*\bs="([^"]+?)")?(?:[^>]*\bht="([^"]+?)")?[^/>]*?>(.*?)</row>|<row\b[^>]*?/>"#,
    )?;

    loop {
        if !eof_reached {
            let n = stream.read(&mut read_buf)?;
            if n == 0 {
                eof_reached = true;
            } else {
                buffer.extend_from_slice(&read_buf[..n]);
            }
        }

        match state {
            SheetStreamingState::SeekWorksheetNode => {
                if let Some(idx) = buffer.find(b"<worksheet") {
                    if let Some(end_idx) = buffer[idx..].find(b">") {
                        let absolute_end = idx + end_idx + 1;
                        output.extend_from_slice(&buffer[..absolute_end]);
                        buffer.drain(..absolute_end);
                        state = SheetStreamingState::InWorksheetNode;
                        continue;
                    }
                }
                if eof_reached {
                    return Err(ShrinkError::InvalidRootNode);
                }
            }
            SheetStreamingState::InWorksheetNode => {
                if let Some(idx) = buffer.find(b"<sheetData") {
                    if let Some(end_idx) = buffer[idx..].find(b">") {
                        let absolute_end = idx + end_idx + 1;
                        output.extend_from_slice(&buffer[..absolute_end]);
                        buffer.drain(..absolute_end);
                        state = SheetStreamingState::InSheetDataNode;
                        continue;
                    }
                } else if buffer.find(b"</worksheet>").is_some() {
                    state = SheetStreamingState::FoundWorksheetEnd;
                    continue;
                }
                if eof_reached && !buffer.is_empty() {
                    output.extend_from_slice(&buffer);
                    buffer.clear();
                    break;
                }
            }
            SheetStreamingState::InSheetDataNode => {
                if let Some(idx) = buffer.find(b"</sheetData>") {
                    let mut chunk = buffer[..idx].to_vec();
                    chunk = process_sheet_chunk(&chunk, params, &row_clean_pattern)?;
                    output.extend_from_slice(&chunk);
                    output.extend_from_slice(b"</sheetData>");
                    buffer.drain(..idx + 12);
                    state = SheetStreamingState::InWorksheetNode;
                } else {
                    if eof_reached {
                        return Err(ShrinkError::UnexpectedEof);
                    }
                    if let Some(last_row_idx) = buffer.rfind(b"</row>") {
                        let split_at = last_row_idx + 6;
                        let mut chunk = buffer[..split_at].to_vec();
                        chunk = process_sheet_chunk(&chunk, params, &row_clean_pattern)?;
                        output.extend_from_slice(&chunk);
                        buffer.drain(..split_at);
                    }
                }
            }
            SheetStreamingState::FoundWorksheetEnd | SheetStreamingState::InColsNode | SheetStreamingState::InMergeCellsNode => {
                output.extend_from_slice(&buffer);
                buffer.clear();
                break;
            }
        }

        if eof_reached && buffer.is_empty() {
            break;
        }
    }

    Ok(output)
}

fn process_sheet_chunk(
    chunk: &[u8],
    _params: &CleanSheetParameters,
    row_pattern: &Regex,
) -> Result<Vec<u8>> {
    if chunk.find(b"<![CDATA[").is_some() {
        return Err(ShrinkError::UnsupportedCData);
    }

    let replaced = row_pattern.replace_all(chunk, |caps: &Captures| {
        let is_hidden = caps.get(2).is_some();
        let inner_content = caps.get(5).map(|m| m.as_bytes()).unwrap_or(b"");

        if inner_content.is_empty() && !is_hidden {
            return Vec::new(); // Leere Reihe komplett entfernen
        }

        caps.get(0).unwrap().as_bytes().to_vec()
    });

    Ok(replaced.into_owned())
}