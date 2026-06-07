#!/usr/bin/env python3
"""
download_nse_bhavcopy_history.py

Downloads all historical NSE equity bhavcopy CSV files.

Two URL formats are used depending on date:
  Old format (2006-01-01 to 2024-07-31):
    https://nsearchives.nseindia.com/content/historical/EQUITIES/{YYYY}/{MON}/cm{DD}{MON}{YYYY}bhav.csv.zip
    Columns: SYMBOL, SERIES, OPEN, HIGH, LOW, CLOSE, LAST, PREVCLOSE, TOTTRDQTY, TOTTRDVAL, TIMESTAMP

  New format (2024-08-01 onwards):
    https://nsearchives.nseindia.com/content/cm/BhavCopy_NSE_CM_0_0_0_{YYYYMMDD}_F_0000.csv.zip
    Columns: 34 detailed columns (TradDt, FinInstrmId, TckrSymb, etc.)

Saves extracted CSVs to: {output-dir}/{YYYY}/NSE_CM_{YYYYMMDD}.csv

Usage:
  python3 download_nse_bhavcopy_history.py [--start YYYY-MM-DD] [--end YYYY-MM-DD]
                                            [--output-dir /path/to/output]
                                            [--delay SECONDS]
"""

import argparse
import os
import io
import sys
import time
import zipfile
from datetime import date, timedelta
from pathlib import Path

import urllib.request
import urllib.error


HEADERS = {
    'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 '
                  '(KHTML, like Gecko) Chrome/114.0.0.0 Safari/537.36',
    'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8',
    'Accept-Language': 'en-US,en;q=0.5',
    'Accept-Encoding': 'gzip, deflate',
    'Connection': 'keep-alive',
}

# Cutover date: old format works up to (and including) 2024-07-31
OLD_FORMAT_CUTOFF = date(2024, 8, 1)

MONTH_MAP = {
    1: 'JAN', 2: 'FEB', 3: 'MAR', 4: 'APR', 5: 'MAY', 6: 'JUN',
    7: 'JUL', 8: 'AUG', 9: 'SEP', 10: 'OCT', 11: 'NOV', 12: 'DEC',
}


def build_url(dt: date) -> str:
    if dt < OLD_FORMAT_CUTOFF:
        mon = MONTH_MAP[dt.month]
        day = dt.strftime('%d')
        yr = dt.strftime('%Y')
        return (f"https://nsearchives.nseindia.com/content/historical/EQUITIES"
                f"/{yr}/{mon}/cm{day}{mon}{yr}bhav.csv.zip")
    else:
        date_str = dt.strftime("%Y%m%d")
        return (f"https://nsearchives.nseindia.com/content/cm"
                f"/BhavCopy_NSE_CM_0_0_0_{date_str}_F_0000.csv.zip")


def download_url(url: str, timeout: int = 30) -> bytes | None:
    req = urllib.request.Request(url, headers=HEADERS)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = resp.read()
            return data
    except urllib.error.HTTPError as e:
        if e.code in (404, 403):
            return None  # No data for this date (weekend/holiday)
        print(f"  HTTP {e.code} for {url}")
        return None
    except Exception as e:
        print(f"  Error fetching {url}: {e}")
        return None


def extract_csv_from_zip(data: bytes) -> tuple[str, str] | None:
    """Extract the first CSV from zip bytes. Returns (filename, csv_content) or None."""
    if not data or data[:1] == b'<':
        return None
    try:
        with zipfile.ZipFile(io.BytesIO(data)) as zf:
            for name in zf.namelist():
                if name.lower().endswith('.csv'):
                    content = zf.read(name).decode('utf-8', errors='replace')
                    return name, content
    except Exception:
        return None
    return None


def generate_working_days(start: date, end: date):
    current = start
    while current <= end:
        if current.weekday() < 5:  # Mon-Fri
            yield current
        current += timedelta(days=1)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--start', default='2006-01-01', help='Start date YYYY-MM-DD')
    parser.add_argument('--end', default=date.today().isoformat(), help='End date YYYY-MM-DD (default: today)')
    parser.add_argument('--output-dir', default='/home/netshare/hdd/downloader/master_data/nse_bhavcopy',
                        help='Directory to save CSV files')
    parser.add_argument('--delay', type=float, default=1.5,
                        help='Delay between requests in seconds (default: 1.5)')
    args = parser.parse_args()

    start_date = date.fromisoformat(args.start)
    end_date = date.fromisoformat(args.end)
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    working_days = list(generate_working_days(start_date, end_date))
    print(f"Downloading NSE bhavcopy for {len(working_days)} potential trading days")
    print(f"Date range: {start_date} to {end_date}")
    print(f"Old format (pre-{OLD_FORMAT_CUTOFF}): /content/historical/EQUITIES/...")
    print(f"New format ({OLD_FORMAT_CUTOFF}+): /content/cm/BhavCopy_NSE_CM_...")
    print(f"Output dir: {output_dir}")
    print(f"Delay: {args.delay}s between requests")
    print()

    downloaded = 0
    skipped_existing = 0
    no_data = 0
    errors = 0

    for i, dt in enumerate(working_days):
        date_str = dt.strftime("%Y%m%d")
        # Save as: output_dir/YYYY/NSE_CM_{YYYYMMDD}.csv
        year_dir = output_dir / str(dt.year)
        year_dir.mkdir(exist_ok=True)
        out_file = year_dir / f"NSE_CM_{date_str}.csv"

        if out_file.exists():
            skipped_existing += 1
            continue

        url = build_url(dt)
        sys.stdout.write(f"[{i+1}/{len(working_days)}] {dt.isoformat()} ... ")
        sys.stdout.flush()

        data = download_url(url)
        if data is None:
            no_data += 1
            sys.stdout.write("no data\n")
            sys.stdout.flush()
            time.sleep(args.delay * 0.5)
            continue

        result = extract_csv_from_zip(data)
        if result is None:
            no_data += 1
            sys.stdout.write("invalid zip\n")
            sys.stdout.flush()
            time.sleep(args.delay * 0.5)
            continue

        _csv_name, csv_content = result
        out_file.write_text(csv_content, encoding='utf-8')
        downloaded += 1
        sys.stdout.write(f"saved ({len(csv_content):,} bytes)\n")
        sys.stdout.flush()

        time.sleep(args.delay)

    print(f"\nDone.")
    print(f"  Downloaded:       {downloaded}")
    print(f"  Already existed:  {skipped_existing}")
    print(f"  No data (holiday/weekend/future): {no_data}")
    print(f"  Errors:           {errors}")


if __name__ == '__main__':
    main()
