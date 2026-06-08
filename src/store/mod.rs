// file: store/mod.rs
// Purpose:
//   Single owner of relational storage for NewsLookout. Centralises SQLite connection
//   creation (WAL + sane pragmas), an additive migration system tracked by a
//   `schema_version` table, and all canonical-schema DDL (roadmap Part C). Previously DDL
//   was scattered across utils.rs / market_data.rs / mod_vectorstore.rs with no migration
//   tracking; new structured-intelligence tables (documents/events/entities/...) and the
//   batch-feed run log live here so cross-table joins are possible and schema evolution is
//   controlled.
//
//   Migrations are embedded string constants applied in ascending `version` order. Each is
//   idempotent (CREATE TABLE IF NOT EXISTS / CREATE INDEX IF NOT EXISTS) and only applied
//   once — `schema_version` records which versions have run. Adding a migration = append a
//   `(version, sql)` tuple to `MIGRATIONS`.

use log::{error, info};
use rusqlite::Connection;

pub mod batch_log;
pub mod batch_writer;
pub mod records;

/// Open (creating if absent) a SQLite database at `db_path` with WAL journaling and the
/// performance/concurrency pragmas the pipeline relies on. Returns an open connection.
pub fn open(db_path: &str) -> Result<Connection, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("open db '{}': {}", db_path, e))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; \
         PRAGMA synchronous=NORMAL; \
         PRAGMA foreign_keys=ON; \
         PRAGMA busy_timeout=10000;",
    )
    .map_err(|e| format!("set pragmas on '{}': {}", db_path, e))?;
    Ok(conn)
}

/// Ordered list of schema migrations. Append-only: never edit or reorder an existing entry.
const MIGRATIONS: &[(i64, &str)] = &[(1, MIGRATION_0001_CANONICAL_SCHEMA)];

/// Open the database and bring it up to the latest schema version.
pub fn open_and_migrate(db_path: &str) -> Result<Connection, String> {
    let conn = open(db_path)?;
    migrate(&conn)?;
    Ok(conn)
}

/// Apply any not-yet-applied migrations, in order, recording each in `schema_version`.
pub fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version    INTEGER PRIMARY KEY,
            applied_ts INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("create schema_version: {}", e))?;

    let current = current_version(conn)?;
    let mut applied = 0usize;
    for (version, sql) in MIGRATIONS {
        if *version > current {
            conn.execute_batch(sql)
                .map_err(|e| format!("apply migration {}: {}", version, e))?;
            conn.execute(
                "INSERT OR REPLACE INTO schema_version (version, applied_ts) VALUES (?1, ?2)",
                rusqlite::params![version, chrono::Utc::now().timestamp()],
            )
            .map_err(|e| format!("record migration {}: {}", version, e))?;
            info!("store: applied schema migration v{}", version);
            applied += 1;
        }
    }
    if applied == 0 {
        info!("store: schema up to date at v{}", current.max(latest_version()));
    }
    Ok(())
}

/// Highest migration version known to this binary.
pub fn latest_version() -> i64 {
    MIGRATIONS.iter().map(|(v, _)| *v).max().unwrap_or(0)
}

/// Highest migration version already applied to this database (0 if none).
pub fn current_version(conn: &Connection) -> Result<i64, String> {
    // schema_version may not exist yet on a brand-new db.
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !exists {
        return Ok(0);
    }
    conn.query_row("SELECT COALESCE(MAX(version), 0) FROM schema_version", [], |r| r.get(0))
        .map_err(|e| format!("read schema_version: {}", e))
}

/// Convenience used at application startup: open + migrate the configured DB, logging
/// (rather than failing the whole app) if migration cannot complete.
pub fn init_at_startup(db_path: &str) {
    match open_and_migrate(db_path) {
        Ok(_) => info!("store: initialised canonical schema at '{}' (v{})", db_path, latest_version()),
        Err(e) => error!("store: could not initialise schema at '{}': {}", db_path, e),
    }
}

// ---------------------------------------------------------------------------
// Migration 0001 — canonical relational schema (roadmap Part C) + batch_run_log.
// All tables keyed off a stable `doc_id` (documents) / `entity_id` (entities). Facts are
// append-only; aggregate rollups are derived elsewhere. Offsets recorded everywhere for
// provenance. Tables are created empty; extractor/emitter plugins populate them in later
// stages.
// ---------------------------------------------------------------------------
const MIGRATION_0001_CANONICAL_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS documents (
    doc_id        TEXT PRIMARY KEY,
    url           TEXT,
    source        TEXT,
    title         TEXT,
    lang          TEXT,
    pubdate_ms    INTEGER,
    pubdate       TEXT,
    plugin        TEXT,
    section       TEXT,
    cluster_id    TEXT,
    tone          REAL,
    word_count    INTEGER,
    ingested_ts   INTEGER
);
CREATE INDEX IF NOT EXISTS idx_documents_pubdate ON documents (pubdate);
CREATE INDEX IF NOT EXISTS idx_documents_cluster ON documents (cluster_id);

CREATE TABLE IF NOT EXISTS events (
    event_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id      TEXT,
    event_type  TEXT,
    actor1      TEXT,
    actor2      TEXT,
    goldstein   REAL,
    quad_class  INTEGER,
    date_ms     INTEGER,
    char_offset INTEGER
);
CREATE INDEX IF NOT EXISTS idx_events_doc ON events (doc_id);
CREATE INDEX IF NOT EXISTS idx_events_type ON events (event_type);

CREATE TABLE IF NOT EXISTS mentions (
    mention_id  INTEGER PRIMARY KEY AUTOINCREMENT,
    cluster_id  TEXT,
    doc_id      TEXT,
    mention_ts  INTEGER,
    source      TEXT,
    confidence  REAL
);
CREATE INDEX IF NOT EXISTS idx_mentions_cluster ON mentions (cluster_id);
CREATE INDEX IF NOT EXISTS idx_mentions_doc ON mentions (doc_id);

CREATE TABLE IF NOT EXISTS entities (
    entity_id      TEXT PRIMARY KEY,
    type           TEXT,
    canonical_name TEXT,
    name_norm      TEXT,
    lei            TEXT,
    cin            TEXT,
    isin           TEXT,
    nse_symbol     TEXT,
    bse_code       TEXT,
    wikidata_qid   TEXT,
    sector         TEXT,
    legal_form     TEXT,
    status         TEXT,
    hq_feature_id  TEXT,
    hq_lat         REAL,
    hq_lon         REAL,
    valid_from     TEXT,
    valid_to       TEXT,
    last_update    TEXT
);
CREATE INDEX IF NOT EXISTS idx_entities_namenorm ON entities (name_norm);
CREATE INDEX IF NOT EXISTS idx_entities_cin ON entities (cin);
CREATE INDEX IF NOT EXISTS idx_entities_isin ON entities (isin);

CREATE TABLE IF NOT EXISTS entity_aliases (
    entity_id  TEXT,
    alias      TEXT,
    alias_norm TEXT,
    alias_type TEXT,
    lang       TEXT
);
CREATE INDEX IF NOT EXISTS idx_aliases_norm ON entity_aliases (alias_norm);
CREATE INDEX IF NOT EXISTS idx_aliases_entity ON entity_aliases (entity_id);

CREATE TABLE IF NOT EXISTS entity_mentions (
    doc_id       TEXT,
    entity_id    TEXT,
    surface_form TEXT,
    char_offset  INTEGER,
    salience     REAL
);
CREATE INDEX IF NOT EXISTS idx_entmen_doc ON entity_mentions (doc_id);
CREATE INDEX IF NOT EXISTS idx_entmen_entity ON entity_mentions (entity_id);

CREATE TABLE IF NOT EXISTS entity_edges (
    edge_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    src_entity_id TEXT,
    dst_entity_id TEXT,
    edge_type     TEXT,
    ownership_pct REAL,
    doc_id        TEXT,
    date_ms       INTEGER,
    tone          REAL,
    theme         TEXT,
    weight        REAL,
    source        TEXT
);
CREATE INDEX IF NOT EXISTS idx_edges_src ON entity_edges (src_entity_id);
CREATE INDEX IF NOT EXISTS idx_edges_dst ON entity_edges (dst_entity_id);

CREATE TABLE IF NOT EXISTS themes (
    doc_id      TEXT,
    theme       TEXT,
    char_offset INTEGER
);
CREATE INDEX IF NOT EXISTS idx_themes_doc ON themes (doc_id);
CREATE INDEX IF NOT EXISTS idx_themes_theme ON themes (theme);

CREATE TABLE IF NOT EXISTS counts (
    doc_id          TEXT,
    count_type      TEXT,
    number          REAL,
    object          TEXT,
    geo_feature_id  TEXT,
    char_offset     INTEGER
);
CREATE INDEX IF NOT EXISTS idx_counts_doc ON counts (doc_id);

CREATE TABLE IF NOT EXISTS amounts (
    doc_id      TEXT,
    value       REAL,
    currency    TEXT,
    unit        TEXT,
    object      TEXT,
    char_offset INTEGER
);
CREATE INDEX IF NOT EXISTS idx_amounts_doc ON amounts (doc_id);

CREATE TABLE IF NOT EXISTS quotes (
    doc_id            TEXT,
    speaker_entity_id TEXT,
    speaker           TEXT,
    verb              TEXT,
    quote             TEXT,
    char_offset       INTEGER
);
CREATE INDEX IF NOT EXISTS idx_quotes_doc ON quotes (doc_id);

CREATE TABLE IF NOT EXISTS dates_ref (
    doc_id      TEXT,
    resolution  TEXT,
    year        INTEGER,
    month       INTEGER,
    day         INTEGER,
    char_offset INTEGER
);
CREATE INDEX IF NOT EXISTS idx_datesref_doc ON dates_ref (doc_id);

CREATE TABLE IF NOT EXISTS gcam (
    doc_id  TEXT,
    dict_id TEXT,
    dim_id  TEXT,
    key     TEXT,
    score   REAL
);
CREATE INDEX IF NOT EXISTS idx_gcam_doc ON gcam (doc_id);

CREATE TABLE IF NOT EXISTS locations (
    doc_id      TEXT,
    name        TEXT,
    feature_id  TEXT,
    lat         REAL,
    lon         REAL,
    country     TEXT,
    adm1        TEXT,
    adm2        TEXT,
    char_offset INTEGER
);
CREATE INDEX IF NOT EXISTS idx_locations_doc ON locations (doc_id);
CREATE INDEX IF NOT EXISTS idx_locations_feature ON locations (feature_id);

CREATE TABLE IF NOT EXISTS market_series (
    source     TEXT,
    instrument TEXT,
    date       TEXT,
    value      REAL,
    unit       TEXT,
    table_ref  TEXT,
    PRIMARY KEY (source, instrument, date)
);
CREATE INDEX IF NOT EXISTS idx_market_series_instr ON market_series (instrument, date);

-- Batch-feed bookkeeping: one row per (source, dataset) recording the last run so the
-- batch CLI can skip re-extraction within a source's frequency window (roadmap point 2f).
CREATE TABLE IF NOT EXISTS batch_run_log (
    source           TEXT NOT NULL,
    dataset          TEXT NOT NULL,
    last_attempt_ts  INTEGER,
    last_success_ts  INTEGER,
    status           TEXT,
    rows_ingested    INTEGER,
    message          TEXT,
    PRIMARY KEY (source, dataset)
);
";

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        c
    }

    #[test]
    fn test_migrate_creates_tables_and_sets_version() {
        let c = mem();
        migrate(&c).unwrap();
        assert_eq!(current_version(&c).unwrap(), latest_version());

        // A representative sample of canonical tables must exist.
        for t in [
            "documents", "events", "mentions", "entities", "entity_aliases",
            "entity_mentions", "entity_edges", "themes", "counts", "amounts",
            "quotes", "dates_ref", "gcam", "locations", "market_series", "batch_run_log",
        ] {
            let found: bool = c
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                    [t],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            assert!(found, "expected table '{}' to exist after migrate", t);
        }
    }

    #[test]
    fn test_migrate_is_idempotent() {
        let c = mem();
        migrate(&c).unwrap();
        // second run must be a no-op and not error
        migrate(&c).unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, latest_version());
    }

    #[test]
    fn test_current_version_zero_on_fresh_db() {
        let c = mem();
        assert_eq!(current_version(&c).unwrap(), 0);
    }
}
