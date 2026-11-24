#!/usr/bin/env python3
"""Enforce >=80% measured production lines per converter source, excluding inline tests."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
if (root / 'src-tauri/src/confluence/convert.rs').is_file():
    expected = {(root / 'src-tauri/src/confluence/convert.rs').resolve()}
else:
    expected = {p.resolve() for p in (root / 'src-tauri/src/converter').rglob('*.rs')}
if not expected:
    raise SystemExit('No production converter sources found')
measured = {}
for record in Path(sys.argv[1]).read_text().split('end_of_record'):
    fields = record.strip().splitlines()
    paths = [line[3:] for line in fields if line.startswith('SF:')]
    if not paths:
        continue
    source = Path(paths[0]).resolve()
    if source not in expected:
        continue
    if source in measured:
        raise SystemExit(f'Duplicate coverage: {source}')
    source_lines = source.read_text().splitlines()
    # Rust source files in these converters keep the inline test module last.
    cutoff = next((i for i, line in enumerate(source_lines, 1) if line.strip() == '#[cfg(test)]'), len(source_lines) + 1)
    hits = {}
    for field in fields:
        if field.startswith('DA:'):
            parts = field[3:].split(',')
            number, count = int(parts[0]), int(parts[1])
            if number < 1 or number > len(source_lines) or count < 0 or number in hits:
                raise SystemExit(f'Invalid/duplicate line coverage: {source}')
            hits[number] = count
    production = [count for number, count in hits.items() if number < cutoff]
    if not production:
        raise SystemExit(f'Empty production coverage: {source}')
    measured[source] = (sum(count > 0 for count in production), len(production))
if set(measured) != expected:
    raise SystemExit(f'Missing converter coverage: {sorted(map(str, expected - set(measured)))}')
failed = False
for source, (covered, total) in sorted(measured.items()):
    print(f'{source.relative_to(root)}: {covered}/{total} ({100 * covered / total:.2f}%) production lines')
    failed |= covered * 100 < total * 80
if failed:
    raise SystemExit('Every converter source must have at least 80% production line coverage')
