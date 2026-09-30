use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Instant, SystemTime};

pub(super) const MAX_LINE: usize = 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const CHECKPOINT: usize = 4096;
#[derive(Default)]
pub(super) struct Cursor {
    pub offset: u64,
    pub size: u64,
    pub initialized: bool,
    pub unreadable: bool,
    pub remainder: Vec<u8>,
    pub reset: bool,
    identity: Option<(u64, u64)>,
    modified: Option<SystemTime>,
    checkpoint: Vec<u8>,
    line_start: u64,
    discarding: bool,
}
fn identity(meta: &fs::Metadata) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}
fn checkpoint(file: &mut File, offset: u64) -> std::io::Result<Vec<u8>> {
    let length = offset.min(CHECKPOINT as u64);
    let mut bytes = vec![0; length as usize];
    file.seek(SeekFrom::Start(offset - length))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
impl Cursor {
    pub fn open(&mut self, path: &Path) -> std::io::Result<Option<File>> {
        self.reset = false;
        let path_meta = fs::symlink_metadata(path)?;
        if !path_meta.is_file() {
            return Err(std::io::Error::other("not a regular log file"));
        }
        if self.initialized
            && self.offset == path_meta.len()
            && self.identity == identity(&path_meta)
            && self.modified == path_meta.modified().ok()
        {
            self.unreadable = false;
            return Ok(None);
        }
        let mut file = File::open(path)?;
        let meta = file.metadata()?;
        let replaced = self.identity != identity(&meta)
            || meta.len() < self.offset
            || (self.offset == meta.len() && self.modified != meta.modified().ok())
            || (self.offset > 0
                && checkpoint(&mut file, self.offset).ok().as_ref() != Some(&self.checkpoint));
        if replaced {
            self.offset = 0;
            self.line_start = 0;
            self.remainder.clear();
            self.discarding = false;
            self.checkpoint.clear();
            self.reset = true;
        }
        self.initialized = true;
        self.unreadable = false;
        self.size = meta.len();
        self.identity = identity(&meta);
        self.modified = meta.modified().ok();
        Ok(Some(file))
    }
    pub fn read(
        &mut self,
        file: &mut File,
        budget: usize,
        line_capacity: usize,
        deadline: Instant,
        mut line: impl FnMut(&[u8], u64),
        oversized: &mut u64,
    ) -> std::io::Result<usize> {
        file.seek(SeekFrom::Start(self.offset))?;
        let mut remaining = self.size.saturating_sub(self.offset).min(budget as u64) as usize;
        let mut buffer = [0; CHUNK];
        let mut read = 0;
        while remaining > 0 && Instant::now() < deadline {
            let count = file.read(&mut buffer[..remaining.min(CHUNK)])?;
            if count == 0 {
                break;
            }
            for part in buffer[..count].split_inclusive(|byte| *byte == b'\n') {
                let complete = part.last() == Some(&b'\n');
                let data = if complete {
                    &part[..part.len() - 1]
                } else {
                    part
                };
                if !self.discarding {
                    if self.remainder.len().saturating_add(data.len()) > line_capacity.min(MAX_LINE)
                    {
                        self.remainder.clear();
                        self.discarding = true;
                        *oversized += 1;
                    } else {
                        self.remainder.extend_from_slice(data);
                    }
                }
                self.offset += part.len() as u64;
                if complete {
                    if !self.discarding {
                        line(&self.remainder, self.line_start);
                    }
                    self.remainder.clear();
                    self.discarding = false;
                    self.line_start = self.offset;
                }
            }
            read += count;
            remaining -= count;
        }
        self.checkpoint = checkpoint(file, self.offset)?;
        Ok(read)
    }
}
