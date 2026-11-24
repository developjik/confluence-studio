#!/usr/bin/env python3
"""Compile a tiny real LLVM coverage case, including same-line/multiline regions."""
from pathlib import Path
import os
import subprocess
import tempfile
import shutil

checker = Path(__file__).with_name('check-converter-coverage.py')
with tempfile.TemporaryDirectory(prefix='converter-llvm-') as folder:
    root = Path(folder)
    source = root / 'src-tauri/src/converter/mod.rs'
    source.parent.mkdir(parents=True)
    source.write_text('''pub fn choose(x: bool) -> i32 {
    let base = if x { 1 } else { 2 };
    let mut total = 0;
    for n in 0..3 { total += n; }
    if x {
        total += base;
    } else {
        total -= base;
    }
    total
}
#[cfg(test)]
mod tests {
    #[test]
    fn both_paths() { assert_eq!(super::choose(true), 4); assert_eq!(super::choose(false), 1); }
}
''')
    (root / 'Cargo.toml').write_text('''[package]
name = "llvm-gate-fixture"
version = "0.0.0"
edition = "2024"
[lib]
path = "src-tauri/src/converter/mod.rs"
[workspace]
''')
    (root / 'scripts').mkdir()
    target = root / 'scripts/check-converter-coverage.py'
    target.write_text(checker.read_text())
    # Do not inherit the parent crate's instrumentation directories.
    env = {k:v for k,v in os.environ.items() if not k.startswith(('CARGO_LLVM_COV_', 'LLVM_PROFILE_FILE', 'CARGO_TARGET_DIR', 'RUSTFLAGS', 'RUSTDOCFLAGS'))}
    evidence = checker.parent.parent / 'converter-llvm-evidence'
    evidence.mkdir(exist_ok=True)
    shutil.copy(source, evidence / 'fixture.rs')
    subprocess.run(['cargo','+stable','llvm-cov','--lcov','--output-path','coverage.lcov'],cwd=root,env=env,check=True)
    subprocess.run(['cargo','+stable','llvm-cov','report','--json','--output-path','coverage.json'],cwd=root,env=env,check=True)
    for name in ['coverage.lcov', 'coverage.json']:
        shutil.copy(root / name, evidence / name)
    shutil.copy(checker, evidence / 'check-converter-coverage.py')
    (evidence / 'replay.py').write_text("""from pathlib import Path
import json, subprocess, tempfile
here = Path(__file__).resolve().parent
with tempfile.TemporaryDirectory() as folder:
    root = Path(folder)
    target = root / 'src-tauri/src/converter/mod.rs'
    target.parent.mkdir(parents=True)
    target.write_bytes((here / 'fixture.rs').read_bytes())
    (root / 'scripts').mkdir()
    gate = root / 'scripts/check-converter-coverage.py'
    gate.write_bytes((here / gate.name).read_bytes())
    data = json.loads((here / 'coverage.json').read_text())
    original = next(item['filename'] for run in data['data'] for item in run['files'] if item['filename'].endswith('/src-tauri/src/converter/mod.rs'))
    # Preserve raw evidence; rewrite only temporary replay copies.
    lcov = (here / 'coverage.lcov').read_text().replace(original, str(target))
    (root / 'coverage.lcov').write_text(lcov)
    fixture_record = next(record for record in lcov.split('end_of_record') if str(target) in record)
    hits = {int(line[3:].split(',')[0]):int(line.split(',')[1]) for line in fixture_record.splitlines() if line.startswith('DA:')}
    assert all(hits.get(line,0)>0 for line in [1,2,3,4,5,6,8,10,11]), hits
    for run in data['data']:
        for item in run['files']:
            if item['filename'] == original: item['filename'] = str(target)
    (root / 'coverage.json').write_text(json.dumps(data))
    subprocess.run(['python3', str(gate), str(root / 'coverage.lcov'), str(root / 'coverage.json')], check=True)
""")
    subprocess.run(['python3', str(evidence / 'replay.py')], check=True)
    # Explicit source-line oracle, independent of the format bridge.
    records = (root / 'coverage.lcov').read_text().split('end_of_record')
    record = next(record for record in records if str(source) in record)
    measured = {int(line[3:].split(',')[0]): int(line.split(',')[1]) for line in record.splitlines() if line.startswith('DA:')}
    assert all(measured.get(line, 0) > 0 for line in [1, 2, 3, 4, 5, 6, 8, 10, 11]), measured
    subprocess.run(['python3',str(target),'coverage.lcov','coverage.json'],cwd=root,check=True)
print('Real LLVM region/LCOV compatibility passed')
