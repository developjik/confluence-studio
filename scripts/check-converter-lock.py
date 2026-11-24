"""Keep every registry dependency in the harness aligned with the desktop lock."""
from pathlib import Path
import re

root = Path(__file__).resolve().parents[1]

def packages(path):
    result = set()
    for block in path.read_text().split('[[package]]')[1:]:
        fields = dict(re.findall(r'^(name|version|source|checksum) = "([^"]*)"$', block, re.M))
        if 'source' in fields:
            result.add(tuple(fields.get(k, '') for k in ('name', 'version', 'source', 'checksum')))
    if not result:
        raise SystemExit(f'No dependency packages in {path}')
    return result

extra = packages(root / 'converter-tests/Cargo.lock') - packages(root / 'src-tauri/Cargo.lock')
if extra:
    raise SystemExit(f'Harness dependencies differ from desktop lock: {sorted(extra)}')
print('Every harness registry dependency matches the desktop lock version/source/checksum')
