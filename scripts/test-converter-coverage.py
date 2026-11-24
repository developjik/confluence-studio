"""Fail-closed regression tests for the coverage gate, no third-party packages."""
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('check-converter-coverage.py').read_text()


class CoverageGateTests(unittest.TestCase):
    def check_report(self, report, expected):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'scripts').mkdir()
            script = root / 'scripts/check-converter-coverage.py'
            script.write_text(SCRIPT)
            source = root / 'src-tauri/src/converter/mod.rs'
            source.parent.mkdir(parents=True)
            source.write_text('fn first() {}\nfn second() {}\n#[cfg(test)]\nmod tests {}\n')
            data = report.replace('SOURCE', str(source))
            path = root / 'coverage.lcov'
            path.write_text(data)
            result = subprocess.run(['python3', str(script), str(path)], capture_output=True, text=True)
            self.assertEqual(result.returncode == 0, expected, result.stdout + result.stderr)

    def test_complete(self):
        self.check_report('SF:SOURCE\nDA:1,1\nDA:2,1\nend_of_record', True)

    def test_tests_do_not_inflate_production(self):
        self.check_report('SF:SOURCE\nDA:1,0\nDA:2,0\nDA:3,100\nDA:4,100\nend_of_record', False)

    def test_low(self):
        self.check_report('SF:SOURCE\nDA:1,1\nDA:2,0\nend_of_record', False)

    def test_missing(self):
        self.check_report('', False)

    def test_empty(self):
        self.check_report('SF:SOURCE\nend_of_record', False)

    def test_duplicate_file(self):
        self.check_report(('SF:SOURCE\nDA:1,1\nend_of_record\n') * 2, False)

    def test_duplicate_line(self):
        self.check_report('SF:SOURCE\nDA:1,1\nDA:1,1\nend_of_record', False)

    def test_invalid_line(self):
        self.check_report('SF:SOURCE\nDA:0,1\nend_of_record', False)

    def test_negative_hit(self):
        self.check_report('SF:SOURCE\nDA:1,-1\nend_of_record', False)


if __name__ == '__main__':
    unittest.main()
