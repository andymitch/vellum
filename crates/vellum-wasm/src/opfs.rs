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
use redb::{ReadableDatabase, ReadableTable, ReadableTableMetadata, StorageBackend};
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

/// Everything durable that isn't the replica: note content, and this device's
/// identity.
///
/// Not an `iroh-blobs` store: that crate has no store trait (stores are actors
/// behind an irpc channel), so a first-class OPFS store means implementing its
/// whole protocol — partial blobs, bao trees, range requests, tags — which
/// belongs upstream. This sits *behind* `MemStore` instead: the durable copy of
/// every blob the replica references, with `MemStore` as the serving layer.
///
/// It is a second redb database on its own OPFS handle, holding two tables.
///
/// `blobs` maps content hash to bytes, which buys the four things the earlier
/// append-only log lacked:
///
///   - **dedup**: the key is the content hash, so re-saving identical content
///     costs nothing and no version is stored twice;
///   - **bounded growth**: `retain` drops blobs no live entry references, the
///     same job the desktop's blob GC does;
///   - **selective load**: `get` is per hash, so boot loads the current
///     entries' content rather than replaying every version ever saved;
///   - **atomic writes**: a redb transaction, rather than "hope the tail isn't
///     torn".
///
/// `meta` holds the two 32-byte keys that make this the *same* device across
/// reloads — the endpoint secret and the author key. Without them every reload
/// mints a new identity, so peers see an endless parade of strangers and every
/// note is authored by someone new. They are here rather than in IndexedDB
/// because the worker already owns this handle, and because a device identity
/// the replica beside it doesn't match is worse than no identity at all.
pub struct VaultStore {
    db: redb::Database,
}

const BLOBS: redb::TableDefinition<&[u8], &[u8]> = redb::TableDefinition::new("blobs");
const META: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("meta");

impl VaultStore {
    pub async fn open(name: &str) -> Result<Self, JsValue> {
        let backend = OpfsBackend::open(name).await?;
        let db = redb::Database::builder()
            .create_with_backend(backend)
            .map_err(|e| JsValue::from_str(&format!("opening the vault store: {e}")))?;
        Ok(Self { db })
    }

    /// One committed transaction per write. Unlike the replica, which batches
    /// entries and commits on a timer, nothing here is left pending — see
    /// `flush` in lib.rs for why that distinction matters in a browser.
    pub fn put_blob(&self, hash: &[u8], bytes: &[u8]) -> anyhow::Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(BLOBS)?;
            t.insert(hash, bytes)?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn get_blob(&self, hash: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let txn = self.db.begin_read()?;
        let t = match txn.open_table(BLOBS) {
            Ok(t) => t,
            // A table nothing has been written to yet does not exist.
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(t.get(hash)?.map(|v| v.value().to_vec()))
    }

    /// Remember a 32-byte key under `name`, or read back what was remembered.
    pub fn put_key(&self, name: &str, key: &[u8; 32]) -> anyhow::Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(META)?;
            t.insert(name, key.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn get_key(&self, name: &str) -> anyhow::Result<Option<[u8; 32]>> {
        let txn = self.db.begin_read()?;
        let t = match txn.open_table(META) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        match t.get(name)? {
            None => Ok(None),
            Some(v) => <[u8; 32]>::try_from(v.value())
                .map(Some)
                .map_err(|_| anyhow::anyhow!("{name} is {} bytes, wanted 32", v.value().len())),
        }
    }

    /// Whether a blob is already stored. Cheaper than `get_blob` when the
    /// answer decides only whether to write.
    pub fn has_blob(&self, hash: &[u8]) -> anyhow::Result<bool> {
        let txn = self.db.begin_read()?;
        let t = match txn.open_table(BLOBS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        Ok(t.get(hash)?.is_some())
    }

    /// Read/write arbitrary text under `name`, in the same `meta` table.
    ///
    /// This is what backs the `SideStore` the vault core asks every shell for:
    /// the peer cache and the local vault-name overrides, which the desktop
    /// keeps as `peers.json` and `vault-names.json` beside its replica.
    pub fn put_text(&self, name: &str, contents: &str) -> anyhow::Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(META)?;
            t.insert(name, contents.as_bytes())?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn get_text(&self, name: &str) -> anyhow::Result<Option<String>> {
        let txn = self.db.begin_read()?;
        let t = match txn.open_table(META) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        match t.get(name)? {
            None => Ok(None),
            Some(v) => Ok(Some(String::from_utf8(v.value().to_vec())?)),
        }
    }

    /// Drop every blob whose hash isn't in `keep`. Returns how many were
    /// dropped and how many remain.
    pub fn retain_blobs(&self, keep: &[Vec<u8>]) -> anyhow::Result<(usize, usize)> {
        let stale: Vec<Vec<u8>> = {
            let txn = self.db.begin_read()?;
            let table = match txn.open_table(BLOBS) {
                Ok(t) => t,
                Err(redb::TableError::TableDoesNotExist(_)) => return Ok((0, 0)),
                Err(e) => return Err(e.into()),
            };
            table
                .iter()?
                .filter_map(|row| row.ok())
                .map(|(k, _)| k.value().to_vec())
                .filter(|hash| !keep.iter().any(|k| k == hash))
                .collect()
        };
        let dropped = stale.len();
        if dropped > 0 {
            let txn = self.db.begin_write()?;
            {
                let mut table = txn.open_table(BLOBS)?;
                for hash in &stale {
                    table.remove(hash.as_slice())?;
                }
            }
            txn.commit()?;
        }
        Ok((dropped, self.blobs_len()?))
    }

    pub fn blobs_len(&self) -> anyhow::Result<usize> {
        let txn = self.db.begin_read()?;
        match txn.open_table(BLOBS) {
            Ok(t) => Ok(t.len()? as usize),
            Err(redb::TableError::TableDoesNotExist(_)) => Ok(0),
            Err(e) => Err(e.into()),
        }
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
