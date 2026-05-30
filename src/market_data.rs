// file: market_data.rs
// Purpose: Save CSV tabular market data (NSE/BSE bhavcopy) to a SQLite database.

use log::{error, info, warn};
use rusqlite::Connection;

/// Sanitize a table name by replacing disallowed characters with underscores.
fn sanitize_table_name(name: &str) -> String {
    name.chars()
        .map(|c| if c == '-' || c == ' ' || c == '.' { '_' } else { c })
        .collect()
}

/// Parse the CSV header line into a vector of trimmed column names.
fn parse_header(header_line: &str) -> Vec<String> {
    header_line
        .split(',')
        .map(|col| col.trim().to_string())
        .collect()
}

/// NSE Capital Market bhavcopy schema:
///   nse_cm_bhavcopy(trade_date, biz_date, segment, src, fin_instrm_tp, fin_instrm_id,
///                   isin, ticker_symbol, security_series, expiry_date, actual_expiry_date,
///                   strike_price, option_type, fin_instrm_name, open_price, high_price,
///                   low_price, close_price, last_price, prev_close, underlying_price,
///                   settlement_price, open_interest, chg_open_interest, total_volume,
///                   total_value, total_trades, session_id, lot_size)
/// Unique key: (trade_date, fin_instrm_id)
///
/// Column mapping from CSV headers:
///   TradDt->trade_date, BizDt->biz_date, Sgmt->segment, Src->src,
///   FinInstrmTp->fin_instrm_tp, FinInstrmId->fin_instrm_id, ISIN->isin,
///   TckrSymb->ticker_symbol, SctySrs->security_series, XpryDt->expiry_date,
///   FininstrmActlXpryDt->actual_expiry_date, StrkPric->strike_price, OptnTp->option_type,
///   FinInstrmNm->fin_instrm_name, OpnPric->open_price, HghPric->high_price,
///   LwPric->low_price, ClsPric->close_price, LastPric->last_price,
///   PrvsClsgPric->prev_close, UndrlygPric->underlying_price, SttlmPric->settlement_price,
///   OpnIntrst->open_interest, ChngInOpnIntrst->chg_open_interest, TtlTradgVol->total_volume,
///   TtlTrfVal->total_value, TtlNbOfTxsExctd->total_trades, SsnId->session_id,
///   NewBrdLotQty->lot_size
const NSE_CREATE_TABLE: &str = "
CREATE TABLE IF NOT EXISTS nse_cm_bhavcopy (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    trade_date          TEXT NOT NULL,
    biz_date            TEXT,
    segment             TEXT,
    src                 TEXT,
    fin_instrm_tp       TEXT,
    fin_instrm_id       INTEGER,
    isin                TEXT,
    ticker_symbol       TEXT,
    security_series     TEXT,
    expiry_date         TEXT,
    actual_expiry_date  TEXT,
    strike_price        REAL,
    option_type         TEXT,
    fin_instrm_name     TEXT,
    open_price          REAL,
    high_price          REAL,
    low_price           REAL,
    close_price         REAL,
    last_price          REAL,
    prev_close          REAL,
    underlying_price    REAL,
    settlement_price    REAL,
    open_interest       REAL,
    chg_open_interest   REAL,
    total_volume        REAL,
    total_value         REAL,
    total_trades        INTEGER,
    session_id          TEXT,
    lot_size            INTEGER
)";

const NSE_CREATE_INDEX_DEDUP: &str =
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_nse_cm_dedup ON nse_cm_bhavcopy (trade_date, fin_instrm_id)";
const NSE_CREATE_INDEX_DATE: &str =
    "CREATE INDEX IF NOT EXISTS idx_nse_cm_date ON nse_cm_bhavcopy (trade_date)";
const NSE_CREATE_INDEX_TICKER: &str =
    "CREATE INDEX IF NOT EXISTS idx_nse_cm_ticker ON nse_cm_bhavcopy (ticker_symbol, trade_date)";
const NSE_CREATE_INDEX_ISIN: &str =
    "CREATE INDEX IF NOT EXISTS idx_nse_cm_isin ON nse_cm_bhavcopy (isin, trade_date)";

const NSE_INSERT: &str = "
INSERT OR IGNORE INTO nse_cm_bhavcopy
    (trade_date, biz_date, segment, src, fin_instrm_tp, fin_instrm_id, isin,
     ticker_symbol, security_series, expiry_date, actual_expiry_date, strike_price,
     option_type, fin_instrm_name, open_price, high_price, low_price, close_price,
     last_price, prev_close, underlying_price, settlement_price, open_interest,
     chg_open_interest, total_volume, total_value, total_trades, session_id, lot_size)
VALUES
    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
     ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)";

fn parse_opt_real(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() { None } else { s.parse::<f64>().ok() }
}

fn parse_opt_int(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() { None } else { s.parse::<f64>().ok().map(|f| f as i64) }
}

/// Save NSE Capital Market bhavcopy CSV content to the `nse_cm_bhavcopy` table.
///
/// The first line of `csv_content` must be the NSE CM header:
///   TradDt,BizDt,Sgmt,Src,FinInstrmTp,FinInstrmId,ISIN,TckrSymb,...
///
/// Returns Ok(rows_inserted) or Err(message).
pub fn save_nse_csv_to_sqlite(csv_content: &str, db_path: &str) -> Result<usize, String> {
    info!("save_nse_csv_to_sqlite: db='{}'", db_path);

    let conn = Connection::open(db_path).map_err(|e| format!("open db '{}': {}", db_path, e))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
        .map_err(|e| format!("pragma: {}", e))?;

    conn.execute(NSE_CREATE_TABLE, [])
        .map_err(|e| format!("create table: {}", e))?;
    for idx_sql in [NSE_CREATE_INDEX_DEDUP, NSE_CREATE_INDEX_DATE, NSE_CREATE_INDEX_TICKER, NSE_CREATE_INDEX_ISIN] {
        conn.execute(idx_sql, []).map_err(|e| format!("create index: {}", e))?;
    }

    let mut lines = csv_content.lines();
    let header = match lines.next() {
        Some(h) if !h.trim().is_empty() => h,
        _ => return Err("NSE CSV has no header".to_string()),
    };

    // Build a column-name to index map so we're robust to column reordering
    let col_names: Vec<&str> = header.split(',').map(|c| c.trim()).collect();
    let col_idx: std::collections::HashMap<&str, usize> = col_names.iter()
        .enumerate()
        .map(|(i, &name)| (name, i))
        .collect();

    macro_rules! get_col {
        ($name:expr, $fields:expr) => {
            col_idx.get($name).and_then(|&i| $fields.get(i)).map(|s| s.trim()).unwrap_or("")
        };
    }

    let mut stmt = conn.prepare(NSE_INSERT).map_err(|e| format!("prepare insert: {}", e))?;
    let mut inserted: usize = 0;
    let expected = col_names.len();

    for (lineno, line) in lines.enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        let fields: Vec<&str> = trimmed.split(',').collect();
        if fields.len() != expected {
            warn!("NSE CSV line {}: expected {} cols, got {} — skipping", lineno + 2, expected, fields.len());
            continue;
        }

        let trade_date = get_col!("TradDt", fields);
        let biz_date   = get_col!("BizDt", fields);
        let segment    = get_col!("Sgmt", fields);
        let src        = get_col!("Src", fields);
        let tp         = get_col!("FinInstrmTp", fields);
        let instrm_id  = parse_opt_int(get_col!("FinInstrmId", fields));
        let isin       = get_col!("ISIN", fields);
        let ticker     = get_col!("TckrSymb", fields);
        let series     = get_col!("SctySrs", fields);
        let expiry     = get_col!("XpryDt", fields);
        let act_expiry = get_col!("FininstrmActlXpryDt", fields);
        let strike     = parse_opt_real(get_col!("StrkPric", fields));
        let opt_type   = get_col!("OptnTp", fields);
        let name       = get_col!("FinInstrmNm", fields);
        let open       = parse_opt_real(get_col!("OpnPric", fields));
        let high       = parse_opt_real(get_col!("HghPric", fields));
        let low        = parse_opt_real(get_col!("LwPric", fields));
        let close      = parse_opt_real(get_col!("ClsPric", fields));
        let last       = parse_opt_real(get_col!("LastPric", fields));
        let prev_close = parse_opt_real(get_col!("PrvsClsgPric", fields));
        let underlying = parse_opt_real(get_col!("UndrlygPric", fields));
        let settlement = parse_opt_real(get_col!("SttlmPric", fields));
        let open_int   = parse_opt_real(get_col!("OpnIntrst", fields));
        let chg_oi     = parse_opt_real(get_col!("ChngInOpnIntrst", fields));
        let volume     = parse_opt_real(get_col!("TtlTradgVol", fields));
        let value      = parse_opt_real(get_col!("TtlTrfVal", fields));
        let trades     = parse_opt_int(get_col!("TtlNbOfTxsExctd", fields));
        let session    = get_col!("SsnId", fields);
        let lot_size   = parse_opt_int(get_col!("NewBrdLotQty", fields));

        if trade_date.is_empty() { continue; }

        match stmt.execute(rusqlite::params![
            trade_date, biz_date, segment, src, tp, instrm_id, isin,
            ticker, series, expiry, act_expiry, strike, opt_type, name,
            open, high, low, close, last, prev_close, underlying, settlement,
            open_int, chg_oi, volume, value, trades, session, lot_size
        ]) {
            Ok(rows) => inserted += rows,
            Err(e) => warn!("NSE insert line {}: {}", lineno + 2, e),
        }
    }

    info!("save_nse_csv_to_sqlite: inserted {} rows.", inserted);
    Ok(inserted)
}

/// Save CSV market data (generic BSE bhavcopy or other) to a SQLite database table.
///
/// # Arguments
/// * `csv_content` - Full CSV text; first line is the header, remaining lines are data rows.
/// * `table_name`  - Destination table name (will be sanitized).
/// * `date`        - Trade date string stored in the `trade_date` column (e.g. `"2024-01-15"`).
/// * `db_path`     - Filesystem path of the SQLite database file (created if absent).
///
/// # Returns
/// `Ok(n)` where `n` is the number of rows inserted, or `Err(msg)` on fatal errors.
pub fn save_csv_to_sqlite(
    csv_content: &str,
    table_name: &str,
    date: &str,
    db_path: &str,
) -> Result<usize, String> {
    let safe_table = sanitize_table_name(table_name);
    info!(
        "save_csv_to_sqlite: table='{}', date='{}', db='{}'",
        safe_table, date, db_path
    );

    let conn = Connection::open(db_path).map_err(|e| {
        error!("Failed to open database '{}': {}", db_path, e);
        format!("Failed to open database '{}': {}", db_path, e)
    })?;

    let mut lines = csv_content.lines();

    let header_line = match lines.next() {
        Some(line) if !line.trim().is_empty() => line,
        _ => {
            error!("CSV content has no header line.");
            return Err("CSV content has no header line.".to_string());
        }
    };

    let columns = parse_header(header_line);
    if columns.is_empty() {
        error!("CSV header is empty.");
        return Err("CSV header is empty.".to_string());
    }

    // For generic CSVs, the dedup key is (trade_date, first_col, second_col) to avoid
    // the case where first_col is the same for all rows (e.g. date columns).
    let dedup_cols = if columns.len() >= 2 {
        format!("trade_date, \"{}\", \"{}\"", columns[0], columns[1])
    } else {
        format!("trade_date, \"{}\"", columns[0])
    };

    let col_defs: String = columns
        .iter()
        .map(|c| format!("\"{}\" TEXT", c))
        .collect::<Vec<_>>()
        .join(", ");

    let create_sql = format!(
        "CREATE TABLE IF NOT EXISTS \"{safe_table}\" \
         (trade_date TEXT NOT NULL, {col_defs})"
    );
    conn.execute(&create_sql, []).map_err(|e| {
        error!("Failed to create table '{}': {}", safe_table, e);
        format!("Failed to create table '{}': {}", safe_table, e)
    })?;

    let index_sql = format!(
        "CREATE UNIQUE INDEX IF NOT EXISTS \"idx_{safe_table}_dedup\" \
         ON \"{safe_table}\" ({dedup_cols})"
    );
    conn.execute(&index_sql, []).map_err(|e| {
        error!("Failed to create unique index on '{}': {}", safe_table, e);
        format!("Failed to create unique index on '{}': {}", safe_table, e)
    })?;

    let placeholders: String = std::iter::repeat_n("?", columns.len() + 1)
        .collect::<Vec<_>>()
        .join(", ");

    let col_names: String = columns
        .iter()
        .map(|c| format!("\"{}\"", c))
        .collect::<Vec<_>>()
        .join(", ");

    let insert_sql = format!(
        "INSERT OR IGNORE INTO \"{safe_table}\" (trade_date, {col_names}) VALUES ({placeholders})"
    );

    let mut stmt = conn.prepare(&insert_sql).map_err(|e| {
        error!("Failed to prepare INSERT statement: {}", e);
        format!("Failed to prepare INSERT statement: {}", e)
    })?;

    let expected_col_count = columns.len();
    let mut inserted: usize = 0;

    for (line_num, line) in lines.enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        let fields: Vec<&str> = trimmed.split(',').collect();
        if fields.len() != expected_col_count {
            warn!(
                "Line {}: expected {} columns, got {} — skipping.",
                line_num + 2, expected_col_count, fields.len()
            );
            continue;
        }

        let params: Vec<&str> = std::iter::once(date)
            .chain(fields.iter().copied())
            .collect();

        match stmt.execute(rusqlite::params_from_iter(params.iter())) {
            Ok(rows_changed) => inserted += rows_changed,
            Err(e) => warn!("Failed to insert line {}: {} — skipping.", line_num + 2, e),
        }
    }

    info!("save_csv_to_sqlite: inserted {} rows into '{}'.", inserted, safe_table);
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_BSE_CSV: &str = "\
SYMBOL,SERIES,OPEN,HIGH,LOW,CLOSE,TOTTRDQTY
RELIANCE,EQ,2800.00,2850.00,2790.00,2840.00,1000000
INFY,EQ,1500.00,1520.00,1490.00,1510.00,500000
TCS,EQ,3500.00,3550.00,3480.00,3530.00,300000";

    const SAMPLE_NSE_CSV: &str = "\
TradDt,BizDt,Sgmt,Src,FinInstrmTp,FinInstrmId,ISIN,TckrSymb,SctySrs,XpryDt,FininstrmActlXpryDt,StrkPric,OptnTp,FinInstrmNm,OpnPric,HghPric,LwPric,ClsPric,LastPric,PrvsClsgPric,UndrlygPric,SttlmPric,OpnIntrst,ChngInOpnIntrst,TtlTradgVol,TtlTrfVal,TtlNbOfTxsExctd,SsnId,NewBrdLotQty,Rmks,Rsvd1,Rsvd2,Rsvd3,Rsvd4
2026-05-29,2026-05-29,CM,NSE,STK,2885,INE002A01018,RELIANCE,EQ,,,,,RELIANCE INDUSTRIES LTD,1360.10,1368.50,1352.40,1356.30,1357.00,1367.00,,1356.30,,,13769747,18716137070.80,148444,F1,1,,,,,
2026-05-29,2026-05-29,CM,NSE,STK,4963,INE009A01021,INFY,EQ,,,,,INFOSYS LTD,1450.00,1465.00,1445.00,1460.00,1460.00,1455.00,,1460.00,,,5000000,7300000000.00,95000,F1,1,,,,,";

    #[test]
    fn test_sanitize_table_name() {
        assert_eq!(sanitize_table_name("NSE-bhavcopy"), "NSE_bhavcopy");
        assert_eq!(sanitize_table_name("BSE bhavcopy"), "BSE_bhavcopy");
        assert_eq!(sanitize_table_name("eq.bhavcopy"), "eq_bhavcopy");
        assert_eq!(sanitize_table_name("clean_name"), "clean_name");
    }

    #[test]
    fn test_parse_header() {
        let cols = parse_header("SYMBOL, SERIES, OPEN, HIGH");
        assert_eq!(cols, vec!["SYMBOL", "SERIES", "OPEN", "HIGH"]);
    }

    #[test]
    fn test_save_bse_csv_to_sqlite() {
        let tmp = std::env::temp_dir().join("bse_market_data_test.db");
        let db_path = tmp.to_str().unwrap();
        let _ = std::fs::remove_file(db_path);

        let result = save_csv_to_sqlite(SAMPLE_BSE_CSV, "BSE-bhavcopy", "2024-01-15", db_path);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        assert_eq!(result.unwrap(), 3);

        // Duplicate import must not insert duplicates.
        let result2 = save_csv_to_sqlite(SAMPLE_BSE_CSV, "BSE-bhavcopy", "2024-01-15", db_path);
        assert!(result2.is_ok());
        assert_eq!(result2.unwrap(), 0, "Duplicate rows should be ignored");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_save_nse_csv_to_sqlite() {
        let tmp = std::env::temp_dir().join("nse_market_data_test.db");
        let db_path = tmp.to_str().unwrap();
        let _ = std::fs::remove_file(db_path);

        let result = save_nse_csv_to_sqlite(SAMPLE_NSE_CSV, db_path);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        assert_eq!(result.unwrap(), 2, "Should insert 2 rows");

        // Duplicate import must be idempotent.
        let result2 = save_nse_csv_to_sqlite(SAMPLE_NSE_CSV, db_path);
        assert!(result2.is_ok());
        assert_eq!(result2.unwrap(), 0, "Duplicate rows should be ignored");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_bad_column_count_rows_skipped() {
        let csv = "SYMBOL,SERIES,CLOSE\nRELIANCE,EQ\nINFY,EQ,1510.00";
        let tmp = std::env::temp_dir().join("market_data_badrow_test.db");
        let db_path = tmp.to_str().unwrap();
        let _ = std::fs::remove_file(db_path);

        let result = save_csv_to_sqlite(csv, "test_table", "2024-01-16", db_path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);

        let _ = std::fs::remove_file(db_path);
    }
}
