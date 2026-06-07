#!/usr/bin/env python3
"""
consolidate_zips.py
Moves zip files from the archive root into year-named subfolders.

Usage:
  python3 consolidate_zips.py --archive-dir /path/to/archive [--dry-run] [--test]

For each YYYY-MM-DD.zip in the archive root:
  - Creates archive_dir/YYYY/ if needed
  - Appends all entries from the root zip into archive_dir/YYYY/YYYY-MM-DD.zip,
    skipping entries that already exist in the destination
  - Deletes the root zip when merging is complete
"""

import argparse
import os
import shutil
import tempfile
import zipfile
from pathlib import Path


def merge_zips(src_path: Path, dst_path: Path, dry_run: bool = False) -> tuple[int, int]:
    """Merge src_path into dst_path, skipping duplicate entries.

    Returns (entries_added, entries_skipped).
    """
    # Read existing entry names from destination (if it exists)
    existing_names: set[str] = set()
    if dst_path.exists():
        with zipfile.ZipFile(dst_path, 'r') as zf:
            existing_names = set(zf.namelist())

    # Read all entries from source
    with zipfile.ZipFile(src_path, 'r') as src_zf:
        src_infos = src_zf.infolist()

        added = 0
        skipped = 0
        new_entries = [info for info in src_infos if info.filename not in existing_names]
        dup_entries = [info for info in src_infos if info.filename in existing_names]
        skipped = len(dup_entries)

        if dry_run:
            print(f"  [DRY-RUN] Would add {len(new_entries)} entries, skip {skipped} duplicates")
            return len(new_entries), skipped

        if new_entries:
            # Write new entries to a temp file in the same directory, then replace
            dst_dir = dst_path.parent
            dst_dir.mkdir(parents=True, exist_ok=True)

            if dst_path.exists():
                # Append mode: copy existing then add new
                tmp_fd, tmp_path = tempfile.mkstemp(dir=dst_dir, suffix='.zip.tmp')
                os.close(tmp_fd)
                tmp_path = Path(tmp_path)
                try:
                    with zipfile.ZipFile(dst_path, 'r') as existing_zf, \
                         zipfile.ZipFile(tmp_path, 'w', compression=zipfile.ZIP_DEFLATED, allowZip64=True) as out_zf:
                        # Copy all existing entries
                        for name in existing_names:
                            try:
                                data = existing_zf.read(name)
                                info = existing_zf.getinfo(name)
                                out_zf.writestr(info, data)
                            except Exception as e:
                                print(f"  WARNING: Could not copy existing entry {name}: {e}")
                        # Add new entries from source
                        for info in new_entries:
                            try:
                                data = src_zf.read(info.filename)
                                out_zf.writestr(info, data)
                                added += 1
                            except Exception as e:
                                print(f"  WARNING: Could not add entry {info.filename}: {e}")
                    # Replace destination with temp file
                    tmp_path.replace(dst_path)
                except Exception as e:
                    tmp_path.unlink(missing_ok=True)
                    raise
            else:
                # Destination doesn't exist: create it with all new entries
                with zipfile.ZipFile(dst_path, 'w', compression=zipfile.ZIP_DEFLATED, allowZip64=True) as out_zf:
                    for info in new_entries:
                        try:
                            data = src_zf.read(info.filename)
                            out_zf.writestr(info, data)
                            added += 1
                        except Exception as e:
                            print(f"  WARNING: Could not add entry {info.filename}: {e}")

    return added, skipped


def consolidate(archive_dir: Path, dry_run: bool = False):
    """Find all YYYY-MM-DD.zip files in the root and merge them into year subfolders."""
    root_zips = sorted(archive_dir.glob("????-??-??.zip"))

    if not root_zips:
        print("No root-level YYYY-MM-DD.zip files found — nothing to do.")
        return

    print(f"Found {len(root_zips)} zip file(s) to consolidate")

    for src_path in root_zips:
        stem = src_path.stem  # e.g. "2026-05-18"
        year = stem[:4]
        dst_dir = archive_dir / year
        dst_path = dst_dir / src_path.name

        src_count = 0
        try:
            with zipfile.ZipFile(src_path, 'r') as zf:
                src_count = len(zf.namelist())
        except Exception as e:
            print(f"  ERROR reading {src_path.name}: {e} — skipping")
            continue

        dst_count = 0
        if dst_path.exists():
            try:
                with zipfile.ZipFile(dst_path, 'r') as zf:
                    dst_count = len(zf.namelist())
            except Exception as e:
                print(f"  WARNING: Could not read destination {dst_path}: {e}")

        print(f"\n{src_path.name}: {src_count} entries in root | {dst_count} entries in {year}/")

        try:
            added, skipped = merge_zips(src_path, dst_path, dry_run=dry_run)
        except Exception as e:
            print(f"  ERROR merging {src_path.name}: {e} — source zip NOT deleted")
            continue

        print(f"  Added {added} new entries, skipped {skipped} duplicates")

        if not dry_run:
            src_path.unlink()
            print(f"  Deleted {src_path.name} from archive root")


def run_test(archive_dir: Path):
    """Test on a copy of one zip from each location (root and subfolder)."""
    import tempfile

    root_zips = sorted(archive_dir.glob("????-??-??.zip"))
    if not root_zips:
        print("No root zip found for test")
        return

    src_path = root_zips[0]
    stem = src_path.stem
    year = stem[:4]
    dst_path = archive_dir / year / src_path.name

    with tempfile.TemporaryDirectory() as tmpdir:
        tmp = Path(tmpdir)
        test_src = tmp / src_path.name
        test_dst = tmp / year / src_path.name

        shutil.copy2(src_path, test_src)
        if dst_path.exists():
            (tmp / year).mkdir()
            shutil.copy2(dst_path, test_dst)

        src_entries = len(zipfile.ZipFile(test_src).namelist())
        dst_entries = len(zipfile.ZipFile(test_dst).namelist()) if test_dst.exists() else 0

        print(f"\n=== TEST on copies of {src_path.name} ===")
        print(f"Source entries: {src_entries}, Dest entries before merge: {dst_entries}")

        added, skipped = merge_zips(test_src, test_dst)

        result_entries = len(zipfile.ZipFile(test_dst).namelist())
        print(f"After merge — dest entries: {result_entries}, added: {added}, skipped: {skipped}")

        # Verify: result should have at least dst_entries entries
        assert result_entries >= dst_entries, "Destination lost entries after merge!"
        # Verify: all entries from source are in result
        src_names = set(zipfile.ZipFile(test_src).namelist())
        dst_names = set(zipfile.ZipFile(test_dst).namelist())
        result_names = set(zipfile.ZipFile(test_dst).namelist())
        for name in src_names:
            assert name in result_names, f"Entry {name} from source missing in merged result!"

        print("TEST PASSED: All source entries present in merged destination.")
        print(f"  Final count: {result_entries} entries (was {dst_entries}, added {added})")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--archive-dir', default='/home/netshare/hdd/downloader/web_articles_archive',
                        help='Path to the archive root directory')
    parser.add_argument('--dry-run', action='store_true', help='Show what would be done without changing anything')
    parser.add_argument('--test', action='store_true', help='Run test on a copy before doing anything')
    args = parser.parse_args()

    archive_dir = Path(args.archive_dir)
    if not archive_dir.is_dir():
        print(f"ERROR: {archive_dir} is not a directory")
        return 1

    if args.test:
        run_test(archive_dir)
        return 0

    consolidate(archive_dir, dry_run=args.dry_run)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
