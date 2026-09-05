//! A redb storage backend on OPFS, so a browser vault survives a reload.
//!
//! redb wants synchronous positional reads and writes; OPFS's
//! `FileSystemSyncAccessHandle` provides exactly that, which is what makes this
//! a thin adapter rather than a re-architecture:
//!
//! | redb `StorageBackend` | sync access handle |
//! | --------------------- | ------------------ |
//! | `len`                 | `getSize`          |
//! | `read(offset, out)`   | `read(buf, { at })`|
//! | `write(offset, data)` | `write(buf, { at })`|
//! | `set_len`             | `truncate`         |
//! | `sync_data`           | `flush`            |
//!
//! Sync access handles are only available in a worker, and they hold an
//! exclusive lock on the file — so exactly one worker owns the vault, which is
//! also where the node should live anyway (iroh's browser build is
//! single-threaded, and CRDT merges on the main thread would jank the editor).

use std::io::{Error, ErrorKind};

use js_sys::Uint8Array;
use redb::StorageBackend;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    FileSystemDirectoryHandle, FileSystemGetFileOptions, FileSystemReadWriteOptions,
    FileSystemSyncAccessHandle,
};

pub struct OpfsBackend {
    handle: FileSystemSyncAccessHandle,
}

// The handle is a JsValue, which is neither Send nor Sync, but redb requires
// both. Sound here because wasm32-unknown-unknown in a browser is
// single-threaded: there is no second thread that could observe it. If iroh's
// browser build ever gains threads (wasm atomics), this has to be revisited.
unsafe impl Send for OpfsBackend {}
unsafe impl Sync for OpfsBackend {}

impl std::fmt::Debug for OpfsBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpfsBackend")
    }
}

/// Open (creating if needed) `name` in the origin's private file system, and
/// take a synchronous access handle on it.
///
/// Async only here, at open time: acquiring the handle is a promise, but every
/// operation on it afterwards is synchronous, which is what redb needs.
/// `navigator.storage` is reached reflectively because web-sys types the
/// worker and window navigators separately.
pub async fn open_handle(name: &str) -> Result<FileSystemSyncAccessHandle, JsValue> {
    let storage = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("navigator"))
        .and_then(|nav| js_sys::Reflect::get(&nav, &JsValue::from_str("storage")))?;
    let dir_promise = js_sys::Reflect::get(&storage, &JsValue::from_str("getDirectory"))?
        .dyn_into::<js_sys::Function>()?
        .call0(&storage)?;
    let dir: FileSystemDirectoryHandle =
        JsFuture::from(js_sys::Promise::from(dir_promise)).await?.dyn_into()?;

    let opts = FileSystemGetFileOptions::new();
    opts.set_create(true);
    let file = JsFuture::from(dir.get_file_handle_with_options(name, &opts))
        .await?
        .dyn_into::<web_sys::FileSystemFileHandle>()?;
    JsFuture::from(file.create_sync_access_handle()).await?.dyn_into()
}

impl OpfsBackend {
    pub async fn open(name: &str) -> Result<Self, JsValue> {
        Ok(Self { handle: open_handle(name).await? })
    }
}

/// Note content, so a reopened vault can read notes and not just list them.
///
/// An append-only log of raw blob bytes, `[u32 length][bytes]…`. It stores no
/// hashes: blobs are content-addressed, so re-adding the bytes on boot
/// reproduces exactly the hashes the replica's entries already point at. Being
/// append-only also means a torn final record costs the last write, never the
/// log — `read_all` stops at the first short record.
///
/// A real port would want something better than replaying every version on
/// boot (blobs are per-save, so this grows), but it is enough to answer whether
/// content can survive at all.
pub struct BlobLog {
    handle: FileSystemSyncAccessHandle,
    end: std::cell::Cell<u64>,
}

unsafe impl Send for BlobLog {}
unsafe impl Sync for BlobLog {}

impl BlobLog {
    pub async fn open(name: &str) -> Result<Self, JsValue> {
        let handle = open_handle(name).await?;
        let end = handle.get_size()? as u64;
        Ok(Self { handle, end: std::cell::Cell::new(end) })
    }

    pub fn append(&self, bytes: &[u8]) -> Result<(), Error> {
        let mut record = Vec::with_capacity(4 + bytes.len());
        record.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        record.extend_from_slice(bytes);
        let opts = FileSystemReadWriteOptions::new();
        opts.set_at(self.end.get() as f64);
        let buf = Uint8Array::from(record.as_slice());
        let written = self
            .handle
            .write_with_buffer_source_and_options(&buf, &opts)
            .map_err(io)? as usize;
        if written != record.len() {
            return Err(Error::new(ErrorKind::WriteZero, "opfs: short blob append"));
        }
        self.end.set(self.end.get() + written as u64);
        self.handle.flush().map_err(io)
    }

    pub fn read_all(&self) -> Result<Vec<Vec<u8>>, Error> {
        let size = self.handle.get_size().map_err(io)? as u64;
        let mut out = Vec::new();
        let mut at = 0u64;
        while at + 4 <= size {
            let mut header = [0u8; 4];
            self.read_exact(at, &mut header)?;
            let len = u32::from_le_bytes(header) as u64;
            if at + 4 + len > size {
                break; // torn final record
            }
            let mut bytes = vec![0u8; len as usize];
            self.read_exact(at + 4, &mut bytes)?;
            out.push(bytes);
            at += 4 + len;
        }
        Ok(out)
    }

    fn read_exact(&self, offset: u64, out: &mut [u8]) -> Result<(), Error> {
        let opts = FileSystemReadWriteOptions::new();
        opts.set_at(offset as f64);
        let buf = Uint8Array::new_with_length(out.len() as u32);
        let read = self
            .handle
            .read_with_buffer_source_and_options(&buf, &opts)
            .map_err(io)? as usize;
        if read != out.len() {
            return Err(Error::new(ErrorKind::UnexpectedEof, "opfs: short blob read"));
        }
        buf.copy_to(out);
        Ok(())
    }
}

fn io(e: JsValue) -> Error {
    Error::new(ErrorKind::Other, format!("opfs: {e:?}"))
}

impl StorageBackend for OpfsBackend {
    fn len(&self) -> Result<u64, Error> {
        self.handle.get_size().map(|n| n as u64).map_err(io)
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), Error> {
        let opts = FileSystemReadWriteOptions::new();
        opts.set_at(offset as f64);
        // read_with_u8_array_and_options copies into a JS view, so read into a
        // temporary and then into `out`. redb's reads are page-sized, so the
        // extra copy is not the bottleneck; correctness first.
        let buf = Uint8Array::new_with_length(out.len() as u32);
        let read = self
            .handle
            .read_with_buffer_source_and_options(&buf, &opts)
            .map_err(io)? as usize;
        if read != out.len() {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                format!("opfs: wanted {} bytes at {offset}, got {read}", out.len()),
            ));
        }
        buf.copy_to(out);
        Ok(())
    }

    fn set_len(&self, len: u64) -> Result<(), Error> {
        self.handle.truncate_with_f64(len as f64).map_err(io)
    }

    fn sync_data(&self) -> Result<(), Error> {
        self.handle.flush().map_err(io)
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), Error> {
        let opts = FileSystemReadWriteOptions::new();
        opts.set_at(offset as f64);
        let buf = Uint8Array::from(data);
        let written = self
            .handle
            .write_with_buffer_source_and_options(&buf, &opts)
            .map_err(io)? as usize;
        if written != data.len() {
            return Err(Error::new(
                ErrorKind::WriteZero,
                format!("opfs: wrote {written} of {} bytes at {offset}", data.len()),
            ));
        }
        Ok(())
    }

    fn close(&self) -> Result<(), Error> {
        self.handle.flush().map_err(io)?;
        self.handle.close();
        Ok(())
    }
}
