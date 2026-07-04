//! Durable cluster state, backed by an embedded [`redb`] database.
//!
//! The store is the source of truth that survives daemon restarts. In-memory
//! [`ClusterState`](crate::state::ClusterState) is hydrated from it on startup
//! and written through on every mutation. redb gives us ACID, single-file,
//! zero-dependency storage — a good fit for control-plane metadata that is
//! small but must not be lost.
//!
//! All redb error types are funnelled through [`HpcError::Store`] so callers
//! only ever deal with the crate-wide [`Result`].

use std::path::Path;
use std::sync::Arc;

use hpc_core::error::{HpcError, Result};
use hpc_core::types::{CommandOutcome, NodeId, NodeRecord};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

/// `node_id -> JSON(NodeRecord)`
const NODES: TableDefinition<&str, &[u8]> = TableDefinition::new("nodes");
/// `command_id -> JSON(CommandOutcome)`
const OUTCOMES: TableDefinition<&str, &[u8]> = TableDefinition::new("command_outcomes");

/// Handle to the persistent state database. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct Store {
    db: Arc<Database>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    /// Open (creating if necessary) the database at `path`, ensuring the parent
    /// directory exists first.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HpcError::io_at(parent, e))?;
        }
        let db = Database::create(path).map_err(HpcError::store)?;
        // Touch both tables so a read on a fresh database doesn't error.
        let wtx = db.begin_write().map_err(HpcError::store)?;
        {
            wtx.open_table(NODES).map_err(HpcError::store)?;
            wtx.open_table(OUTCOMES).map_err(HpcError::store)?;
        }
        wtx.commit().map_err(HpcError::store)?;
        tracing::info!(path = %path.display(), "opened state store");
        Ok(Store { db: Arc::new(db) })
    }

    /// Insert or update a node record.
    pub fn put_node(&self, record: &NodeRecord) -> Result<()> {
        let bytes = serde_json::to_vec(record)?;
        let wtx = self.db.begin_write().map_err(HpcError::store)?;
        {
            let mut table = wtx.open_table(NODES).map_err(HpcError::store)?;
            table
                .insert(record.info.node_id.as_str(), bytes.as_slice())
                .map_err(HpcError::store)?;
        }
        wtx.commit().map_err(HpcError::store)?;
        Ok(())
    }

    /// Remove a node record. Returns `true` if a record was present.
    pub fn delete_node(&self, node_id: &NodeId) -> Result<bool> {
        let wtx = self.db.begin_write().map_err(HpcError::store)?;
        let existed;
        {
            let mut table = wtx.open_table(NODES).map_err(HpcError::store)?;
            existed = table
                .remove(node_id.as_str())
                .map_err(HpcError::store)?
                .is_some();
        }
        wtx.commit().map_err(HpcError::store)?;
        Ok(existed)
    }

    /// Load every persisted node record (used to hydrate memory on startup).
    pub fn load_nodes(&self) -> Result<Vec<NodeRecord>> {
        let rtx = self.db.begin_read().map_err(HpcError::store)?;
        let table = rtx.open_table(NODES).map_err(HpcError::store)?;
        let mut out = Vec::new();
        for entry in table.iter().map_err(HpcError::store)? {
            let (_key, value) = entry.map_err(HpcError::store)?;
            let record: NodeRecord = serde_json::from_slice(value.value())?;
            out.push(record);
        }
        Ok(out)
    }

    /// Append a command outcome to the audit log.
    pub fn put_outcome(&self, outcome: &CommandOutcome) -> Result<()> {
        let bytes = serde_json::to_vec(outcome)?;
        let wtx = self.db.begin_write().map_err(HpcError::store)?;
        {
            let mut table = wtx.open_table(OUTCOMES).map_err(HpcError::store)?;
            table
                .insert(outcome.command_id.as_str(), bytes.as_slice())
                .map_err(HpcError::store)?;
        }
        wtx.commit().map_err(HpcError::store)?;
        Ok(())
    }

    /// Load all recorded command outcomes, most-recently-completed first.
    pub fn load_outcomes(&self) -> Result<Vec<CommandOutcome>> {
        let rtx = self.db.begin_read().map_err(HpcError::store)?;
        let table = rtx.open_table(OUTCOMES).map_err(HpcError::store)?;
        let mut out = Vec::new();
        for entry in table.iter().map_err(HpcError::store)? {
            let (_key, value) = entry.map_err(HpcError::store)?;
            out.push(serde_json::from_slice::<CommandOutcome>(value.value())?);
        }
        out.sort_by_key(|o| std::cmp::Reverse(o.completed_at_unix));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hpc_core::types::{NodeInfo, NodeRole};

    fn sample_record(id: &str) -> NodeRecord {
        NodeRecord::new(NodeInfo {
            node_id: id.to_string(),
            hostname: id.to_string(),
            ip_address: "10.0.0.1".into(),
            role: NodeRole::Storage,
            cpu_cores: 8,
            total_memory_bytes: 1 << 34,
            total_disk_bytes: 1 << 40,
            agent_version: "0.1.0".into(),
            kernel_version: "6.1".into(),
            os: "linux".into(),
            started_at_unix: 0,
            labels: Default::default(),
        })
    }

    #[test]
    fn roundtrips_nodes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(dir.path().join("s.redb")).expect("open");
        store.put_node(&sample_record("n1")).expect("put");
        store.put_node(&sample_record("n2")).expect("put");
        let loaded = store.load_nodes().expect("load");
        assert_eq!(loaded.len(), 2);
        assert!(store.delete_node(&"n1".to_string()).expect("delete"));
        assert_eq!(store.load_nodes().expect("load").len(), 1);
    }
}
