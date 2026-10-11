// Chunked reading of output tables: Connection::call_chunked and TableReader.

use std::sync::{Arc, Mutex};

use indexmap::IndexMap;

use crate::connection::{CompatOptions, FuncHandle};
use crate::error::RfcError;
use crate::ffi;
use crate::marshal::{self, OutputSlot};
use crate::mem;
use crate::value::RfcValue;

/// One output parameter of [`Connection::call_chunked`](crate::Connection::call_chunked).
pub enum ChunkedOutput {
    /// Every output that is not a top-level table, read in full — the same
    /// value `Connection::call` returns for it.
    Value(RfcValue),
    /// A top-level table output, still in the SDK container.
    Table(TableReader),
}

/// A top-level output table left in the SDK container by
/// [`Connection::call_chunked`](crate::Connection::call_chunked), taken in
/// chunks from the last row backwards.
///
/// [`take_last_rows`](Self::take_last_rows) reads the last rows not yet
/// taken, in their original order, then deletes them from the container.
/// The caller loops until [`remaining`](Self::remaining) is 0 and places the
/// chunks itself: the first chunk holds the LAST rows of the table. Rows
/// are the same maps `Connection::call` puts in `RfcValue::Table`.
///
/// The reader shares ownership of the function container with the other
/// readers of the same call; the container is destroyed when the last of
/// them is dropped, whether or not every row was taken. It does not borrow
/// the connection — reading a container makes no network round-trip.
///
/// `Send` but not `Sync`, like `Connection`: the SDK forbids using one
/// container from two threads at the same time. Every SDK access holds the
/// container lock, so two readers of the same call can live on different
/// threads; a chunk on one waits for a chunk on the other.
pub struct TableReader {
    container: Arc<Mutex<Container>>,
    table: TableCursor,
    rows: u32,
    remaining: u32,
}

/// The raw table and row-type handles of one reader.
struct TableCursor {
    handle: ffi::RFC_TABLE_HANDLE,
    type_desc: ffi::RFC_TYPE_DESC_HANDLE,
}

// Safety: both are pointers to SDK objects not tied to a thread. The table
// handle belongs to the container in `TableReader::container` and is only
// used while that container's lock is held. The type description is
// immutable metadata from the SDK's repository cache, which the guide
// allows to be shared across threads; the core never clears that cache.
unsafe impl Send for TableCursor {}

/// The function container the readers of one call share. Destroying it
/// frees every row still in it, so the free pages are returned to the
/// operating system right after.
struct Container {
    func: Option<FuncHandle>,
}

impl Drop for Container {
    fn drop(&mut self) {
        // Destroy the container first; the trim can only return pages that
        // are already free.
        drop(self.func.take());
        mem::release_free_memory();
    }
}

impl TableReader {
    /// Number of rows the call returned in this table.
    pub fn row_count(&self) -> usize {
        self.rows as usize
    }

    /// Number of rows not yet taken.
    pub fn remaining(&self) -> usize {
        self.remaining as usize
    }

    /// Read the last `max_rows` rows not yet taken (fewer when fewer
    /// remain), in their original order, then delete exactly those rows
    /// from the SDK table. Returns an empty chunk when the table is drained
    /// or `max_rows` is 0.
    ///
    /// All rows of the chunk are read before any is deleted, so a read
    /// error leaves the table as it was. A delete error leaves the rows
    /// before it deleted; `remaining` counts what is still in the table.
    /// The rows of a chunk that fails are not returned. After the deletes
    /// the free pages are returned to the operating system.
    pub fn take_last_rows(
        &mut self,
        max_rows: usize,
    ) -> Result<Vec<IndexMap<String, RfcValue>>, RfcError> {
        let n = self
            .remaining
            .min(u32::try_from(max_rows).unwrap_or(u32::MAX));
        if n == 0 {
            return Ok(Vec::new());
        }
        let sdk = ffi::sdk()?;
        let container = self
            .container
            .lock()
            .expect("function container mutex poisoned");
        let start = self.remaining - n;
        let rows = marshal::read_rows(
            sdk,
            self.table.handle,
            self.table.type_desc,
            start..self.remaining,
        )?;
        for _ in 0..n {
            marshal::delete_last_row(sdk, self.table.handle)?;
            self.remaining -= 1;
        }
        drop(container);
        mem::release_free_memory();
        Ok(rows)
    }
}

/// Read the outputs of an invoked container: tables become readers that
/// share the container, everything else is read now. The container is
/// destroyed on return when no table output holds it.
pub(crate) fn open_outputs(
    func_desc: ffi::RFC_FUNCTION_DESC_HANDLE,
    func_handle: FuncHandle,
    compat: CompatOptions,
) -> Result<IndexMap<String, ChunkedOutput>, RfcError> {
    let sdk = ffi::sdk()?;
    let slots = marshal::open_outputs(sdk, func_desc, func_handle.raw(), compat)?;
    let container = Arc::new(Mutex::new(Container {
        func: Some(func_handle),
    }));
    Ok(slots
        .into_iter()
        .map(|(name, slot)| {
            let output = match slot {
                OutputSlot::Value(v) => ChunkedOutput::Value(v),
                OutputSlot::Table {
                    handle,
                    type_desc,
                    rows,
                } => ChunkedOutput::Table(TableReader {
                    container: Arc::clone(&container),
                    table: TableCursor { handle, type_desc },
                    rows,
                    remaining: rows,
                }),
            };
            (name, output)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_move_between_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<TableReader>();
        assert_send::<ChunkedOutput>();
        assert_send::<IndexMap<String, ChunkedOutput>>();
    }
}
