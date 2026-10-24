"""Fast source checks that do not download or compile Chromium."""
import ast
import json
import os
from pathlib import Path
import re
import subprocess

REPO = Path(__file__).resolve().parents[1]


def main():
    paths = subprocess.check_output(['git', 'ls-files', '-z'], cwd=REPO).decode().split('\0')
    for name in filter(None, paths):
        path = REPO / name
        if not path.is_file():
            continue
        if path.suffix == '.py':
            ast.parse(path.read_text(encoding='utf-8'), filename=name)
        elif path.suffix == '.cjs':
            subprocess.run(['node', '--check', str(path)], check=True)
        elif path.suffix == '.sh' and os.name != 'nt':
            subprocess.run(['bash', '-n', str(path)], check=True)
    config = json.loads((REPO / 'windows/chromium.json').read_text())
    assert re.fullmatch(r'\d+\.\d+\.\d+\.\d+', config['version'])
    assert config['ungoogled_release'].startswith(config['version'] + '-')
    for key in ('revision', 'ungoogled_revision'):
        assert re.fullmatch(r'[a-f0-9]{40}', config[key]), key
    print('Python, JavaScript, shell syntax and Chromium manifest checked.')


if __name__ == '__main__':
    main()
