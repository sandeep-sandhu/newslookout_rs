#!/usr/bin/env python3
"""
load_nse_to_sqlite.py

Creates and populates a SQLite database with NSE Capital Market bhavcopy data.

Handles two CSV formats automatically:

Old format (pre-Aug 2024): SYMBOL, SERIES, OPEN, HIGH, LOW, CLOSE, LAST, PREVCLOSE,
  TOTTRDQTY, TOTTRDVAL, TIMESTAMP  → table nse_cm_bhavcopy_legacy

New format (Aug 2024+): 34 columns with TradDt, FinInstrmId, TckrSymb, etc.
  → table nse_cm_bhavcopy

Usage:
  python3 load_nse_to_sqlite.py --csv-dir /path/to/nse_bhavcopy --db /path/to/market_data.db
  python3 load_nse_to_sqlite.py --csv-file /path/to/file.csv --db /path/to/market_data.db
"""

import argparse
import csv
import io
import re
import sqlite3
import sys
from pathlib import Path

# ─── New-format schema (Aug 2024+) ────────────────────────────────────────────

CREATE_NEW_TABLE_SQL = """
CREATE TABLE IF NOT EXISTS nse_cm_bhavcopy (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    trade_date     TEXT NOT NULL,
    biz_date       TEXT,
    segment        TEXT,
    src            TEXT,
    fin_instrm_tp  TEXT,
    fin_instrm_id  INTEGER,
    isin           TEXT,
    ticker_symbol  TEXT,
    security_series TEXT,
    expiry_date    TEXT,
    actual_expiry_date TEXT,
    strike_price   REAL,
    option_type    TEXT,
    fin_instrm_name TEXT,
    open_price     REAL,
    high_price     REAL,
    low_price      REAL,
    close_price    REAL,
    last_price     REAL,
    prev_close     REAL,
    underlying_price REAL,
    settlement_price REAL,
    open_interest  REAL,
    chg_open_interest REAL,
    total_volume   REAL,
    total_value    REAL,
    total_trades   INTEGER,
    session_id     TEXT,
    lot_size       INTEGER
);
"""

CREATE_NEW_INDEXES = [
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_nse_cm_dedup ON nse_cm_bhavcopy (trade_date, fin_instrm_id);",
    "CREATE INDEX IF NOT EXISTS idx_nse_cm_date ON nse_cm_bhavcopy (trade_date);",
    "CREATE INDEX IF NOT EXISTS idx_nse_cm_ticker ON nse_cm_bhavcopy (ticker_symbol, trade_date);",
    "CREATE INDEX IF NOT EXISTS idx_nse_cm_isin ON nse_cm_bhavcopy (isin, trade_date);",
]

NEW_COL_MAP = {
    'TradDt': 'trade_date', 'BizDt': 'biz_date', 'Sgmt': 'segment', 'Src': 'src',
    'FinInstrmTp': 'fin_instrm_tp', 'FinInstrmId': 'fin_instrm_id', 'ISIN': 'isin',
    'TckrSymb': 'ticker_symbol', 'SctySrs': 'security_series', 'XpryDt': 'expiry_date',
    'FininstrmActlXpryDt': 'actual_expiry_date', 'StrkPric': 'strike_price',
    'OptnTp': 'option_type', 'FinInstrmNm': 'fin_instrm_name', 'OpnPric': 'open_price',
    'HghPric': 'high_price', 'LwPric': 'low_price', 'ClsPric': 'close_price',
    'LastPric': 'last_price', 'PrvsClsgPric': 'prev_close', 'UndrlygPric': 'underlying_price',
    'SttlmPric': 'settlement_price', 'OpnIntrst': 'open_interest',
    'ChngInOpnIntrst': 'chg_open_interest', 'TtlTradgVol': 'total_volume',
    'TtlTrfVal': 'total_value', 'TtlNbOfTxsExctd': 'total_trades',
    'SsnId': 'session_id', 'NewBrdLotQty': 'lot_size',
}

NEW_INTEGER_COLS = {'fin_instrm_id', 'total_trades', 'lot_size'}
NEW_REAL_COLS = {
    'strike_price', 'open_price', 'high_price', 'low_price', 'close_price',
    'last_price', 'prev_close', 'underlying_price', 'settlement_price',
    'open_interest', 'chg_open_interest', 'total_volume', 'total_value',
}

# ─── Legacy-format schema (2006 – Jul 2024) ───────────────────────────────────

CREATE_LEGACY_TABLE_SQL = """
CREATE TABLE IF NOT EXISTS nse_cm_bhavcopy_legacy (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    trade_date   TEXT NOT NULL,    -- TIMESTAMP field, normalised to YYYY-MM-DD
    ticker_symbol TEXT NOT NULL,   -- SYMBOL
    series       TEXT,             -- SERIES
    open_price   REAL,             -- OPEN
    high_price   REAL,             -- HIGH
    low_price    REAL,             -- LOW
    close_price  REAL,             -- CLOSE
    last_price   REAL,             -- LAST
    prev_close   REAL,             -- PREVCLOSE
    total_volume REAL,             -- TOTTRDQTY
    total_value  REAL              -- TOTTRDVAL
);
"""

CREATE_LEGACY_INDEXES = [
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_nse_leg_dedup ON nse_cm_bhavcopy_legacy (trade_date, ticker_symbol, series);",
    "CREATE INDEX IF NOT EXISTS idx_nse_leg_date ON nse_cm_bhavcopy_legacy (trade_date);",
    "CREATE INDEX IF NOT EXISTS idx_nse_leg_ticker ON nse_cm_bhavcopy_legacy (ticker_symbol, trade_date);",
]

LEGACY_MONTH_MAP = {
    'JAN': '01', 'FEB': '02', 'MAR': '03', 'APR': '04', 'MAY': '05', 'JUN': '06',
    'JUL': '07', 'AUG': '08', 'SEP': '09', 'OCT': '10', 'NOV': '11', 'DEC': '12',
}


def parse_legacy_date(ts: str) -> str:
    """Convert '2-JAN-2006' → '2006-01-02'."""
    ts = ts.strip()
    m = re.match(r'(\d{1,2})-([A-Z]{3})-(\d{4})', ts)
    if m:
        day, mon, yr = m.group(1), m.group(2), m.group(3)
        return f"{yr}-{LEGACY_MONTH_MAP.get(mon, '00')}-{day.zfill(2)}"
    return ts


# ─── Helpers ──────────────────────────────────────────────────────────────────

def coerce_num(val: str, as_int: bool = False):
    val = val.strip()
    if not val:
        return None
    try:
        f = float(val)
        return int(f) if as_int else f
    except ValueError:
        return None


def detect_format(fieldnames: list[str]) -> str:
    """Return 'new' for post-Aug-2024 format, 'legacy' for old format, 'unknown' otherwise."""
    if 'TradDt' in fieldnames or 'FinInstrmId' in fieldnames:
        return 'new'
    if 'SYMBOL' in fieldnames and 'TIMESTAMP' in fieldnames:
        return 'legacy'
    return 'unknown'


def setup_db(conn: sqlite3.Connection):
    conn.execute(CREATE_NEW_TABLE_SQL)
    for s in CREATE_NEW_INDEXES:
        conn.execute(s)
    conn.execute(CREATE_LEGACY_TABLE_SQL)
    for s in CREATE_LEGACY_INDEXES:
        conn.execute(s)
    conn.commit()


# ─── New-format loader ────────────────────────────────────────────────────────

def load_new_format(csv_content: str, conn: sqlite3.Connection) -> tuple[int, int]:
    reader = csv.DictReader(io.StringIO(csv_content))
    if not reader.fieldnames:
        return 0, 0

    fieldnames = [f.strip() for f in reader.fieldnames if f and f.strip()]
    db_cols = [NEW_COL_MAP[f] for f in fieldnames if f in NEW_COL_MAP]
    csv_cols = [f for f in fieldnames if f in NEW_COL_MAP]
    if not db_cols:
        return 0, 0

    placeholders = ', '.join('?' for _ in db_cols)
    col_list = ', '.join(db_cols)
    insert_sql = f"INSERT OR IGNORE INTO nse_cm_bhavcopy ({col_list}) VALUES ({placeholders})"

    rows = []
    for row in reader:
        values = []
        for col in csv_cols:
            db_col = NEW_COL_MAP[col]
            raw = row.get(col, '').strip()
            if db_col in NEW_INTEGER_COLS:
                values.append(coerce_num(raw, as_int=True))
            elif db_col in NEW_REAL_COLS:
                values.append(coerce_num(raw))
            else:
                values.append(raw if raw else None)
        rows.append(values)

    if not rows:
        return 0, 0
    cursor = conn.executemany(insert_sql, rows)
    conn.commit()
    inserted = cursor.rowcount if cursor.rowcount >= 0 else len(rows)
    return inserted, len(rows) - inserted


# ─── Legacy-format loader ─────────────────────────────────────────────────────

def load_legacy_format(csv_content: str, conn: sqlite3.Connection) -> tuple[int, int]:
    reader = csv.DictReader(io.StringIO(csv_content))
    if not reader.fieldnames:
        return 0, 0

    insert_sql = """
        INSERT OR IGNORE INTO nse_cm_bhavcopy_legacy
            (trade_date, ticker_symbol, series, open_price, high_price, low_price,
             close_price, last_price, prev_close, total_volume, total_value)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """

    rows = []
    for row in reader:
        ts = row.get('TIMESTAMP', '').strip()
        if not ts:
            continue
        rows.append((
            parse_legacy_date(ts),
            row.get('SYMBOL', '').strip(),
            row.get('SERIES', '').strip(),
            coerce_num(row.get('OPEN', '')),
            coerce_num(row.get('HIGH', '')),
            coerce_num(row.get('LOW', '')),
            coerce_num(row.get('CLOSE', '')),
            coerce_num(row.get('LAST', '')),
            coerce_num(row.get('PREVCLOSE', '')),
            coerce_num(row.get('TOTTRDQTY', '')),
            coerce_num(row.get('TOTTRDVAL', '')),
        ))

    if not rows:
        return 0, 0
    cursor = conn.executemany(insert_sql, rows)
    conn.commit()
    inserted = cursor.rowcount if cursor.rowcount >= 0 else len(rows)
    return inserted, len(rows) - inserted


def load_csv_content(csv_content: str, conn: sqlite3.Connection) -> tuple[int, int, str]:
    """Detect format and load. Returns (inserted, skipped, format_name)."""
    reader = csv.DictReader(io.StringIO(csv_content))
    fieldnames = list(reader.fieldnames or [])
    fmt = detect_format(fieldnames)
    if fmt == 'new':
        ins, skp = load_new_format(csv_content, conn)
    elif fmt == 'legacy':
        ins, skp = load_legacy_format(csv_content, conn)
    else:
        ins, skp = 0, 0
    return ins, skp, fmt


def load_csv_file(csv_path: Path, conn: sqlite3.Connection) -> tuple[int, int, str]:
    content = csv_path.read_text(encoding='utf-8', errors='replace')
    return load_csv_content(content, conn)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--db', default='/home/netshare/hdd/downloader/market_data.db')
    parser.add_argument('--csv-dir', help='Directory containing NSE CSV files (scanned recursively)')
    parser.add_argument('--csv-file', help='Single CSV file to load')
    parser.add_argument('--verbose', '-v', action='store_true')
    args = parser.parse_args()

    if not args.csv_dir and not args.csv_file:
        parser.error("Provide --csv-dir or --csv-file")

    conn = sqlite3.connect(args.db)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    setup_db(conn)

    if args.csv_file:
        csv_files = [Path(args.csv_file)]
    else:
        csv_files = sorted(Path(args.csv_dir).rglob("*.csv"))

    print(f"Loading {len(csv_files)} file(s) into {args.db}")

    total_inserted = 0
    total_skipped = 0
    new_count = 0
    legacy_count = 0
    unknown_count = 0

    for i, csv_path in enumerate(csv_files, 1):
        if not csv_path.exists():
            print(f"  SKIP (not found): {csv_path}")
            continue
        inserted, skipped, fmt = load_csv_file(csv_path, conn)
        total_inserted += inserted
        total_skipped += skipped
        if fmt == 'new':
            new_count += 1
        elif fmt == 'legacy':
            legacy_count += 1
        else:
            unknown_count += 1
        if args.verbose or i % 200 == 0:
            print(f"  [{i}/{len(csv_files)}] {csv_path.name} ({fmt}): +{inserted} rows ({skipped} dupes)")

    conn.close()
    print(f"\nDone.")
    print(f"  Files:          {i} total ({new_count} new-format, {legacy_count} legacy, {unknown_count} unknown)")
    print(f"  Rows inserted:  {total_inserted:,}")
    print(f"  Rows skipped:   {total_skipped:,}")


if __name__ == '__main__':
    main()
