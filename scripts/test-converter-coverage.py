"""Fail-closed coverage gate regression tests, standard library only."""
from pathlib import Path
import json
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('check-converter-coverage.py').read_text()


class CoverageGateTests(unittest.TestCase):
    def run_gate(self, counts, *, expected=False, truncate=False, missing=False,
                 duplicate=False, totals_delta=0, remove_line=False):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'scripts').mkdir()
            script = root / 'scripts/check-converter-coverage.py'
            script.write_text(SCRIPT)
            source = root / 'src-tauri/src/converter/mod.rs'
            source.parent.mkdir(parents=True)
            source.write_text('fn a() {}\nfn b() {}\n#[cfg(test)]\n' + 'test line\n' * 9)
            segments = []
            for number, count in enumerate(counts, 1):
                segments.extend([[number, 1, count, True, True, False],
                                 [number + 1, 1, 0, False, False, False]])
            summary = {'count': len(counts), 'covered': sum(c > 0 for c in counts)}
            report = {'data':[{'files':[] if missing else [{'filename':str(source), 'segments':segments, 'summary':{'lines':summary}}]}]}
            rows = [f'DA:{i},{c}' for i,c in enumerate(counts, 1)]
            if remove_line:
                rows.pop()
            record = f'SF:{source}\n' + '\n'.join(rows) + f'\nLF:{len(counts)+totals_delta}\nLH:{summary["covered"]}\nend_of_record\n'
            if truncate:
                record = record.replace('end_of_record', '')
            if duplicate:
                record *= 2
            path = root / 'coverage.lcov'
            path.write_text('' if missing else record)
            data = root / 'coverage.json'
            data.write_text(json.dumps(report))
            result = subprocess.run(['python3',str(script),str(path),str(data)],capture_output=True,text=True)
            self.assertEqual(result.returncode == 0, expected, result.stdout + result.stderr)

    def test_complete(self): self.run_gate([1,1], expected=True)
    def test_low(self): self.run_gate([1,0])
    def test_tests_do_not_inflate_production(self):
        # Including tests would pass 10/12=83.3%; excluding them must fail 0/2.
        self.run_gate([0,0]+[1]*10)
    def test_uncovered_tests_do_not_reduce_production(self):
        self.run_gate([1,1]+[0]*10, expected=True)
    def test_missing(self): self.run_gate([1,1],missing=True)
    def test_empty(self): self.run_gate([])
    def test_truncated(self): self.run_gate([1,1],truncate=True)
    def test_duplicate(self): self.run_gate([1,1],duplicate=True)
    def test_inconsistent_totals(self): self.run_gate([1,1],totals_delta=1)
    def test_missing_da(self): self.run_gate([1,1],remove_line=True)
    def test_negative_hit(self): self.run_gate([1,-1])


if __name__ == '__main__': unittest.main()
