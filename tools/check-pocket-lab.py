"""Run after installing Pocket Lab and downloading Gemma 4 E2B: python tools/check-pocket-lab.py [cpu|gpu]."""
import json
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
lab = root / '.local/data/pocket-lab'
backend = sys.argv[1] if len(sys.argv) > 1 else 'cpu'
assert backend in ('cpu', 'gpu')
(lab / 'cache').mkdir(exist_ok=True)
request = dict(mode='chat', modelPath=str(lab / 'models/gemma-4-E2B-it.litertlm'), cachePath=str(lab / 'cache'),
    config=dict(systemPrompt='Answer briefly.', contextTokens=1024, maxTokens=64, topK=40, topP=.95, temperature=.2, accelerator=backend, thinking=backend == 'gpu', speculative=backend == 'gpu'),
    messages=[dict(role='user', text='What is 2 + 2? Answer briefly.')])
result = subprocess.run([str(lab / 'runtime/Scripts/python.exe'), '-u', str(root / 'app/src-tauri/src/pocket_runtime.py')], input=json.dumps(request)+'\n', capture_output=True, text=True, timeout=300)
chunks = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
assert result.returncode == 0 and chunks and chunks[-1].get('done'), (result.stdout, result.stderr)
answer = ''.join(row.get('text', '') for row in chunks)
assert '4' in answer or 'four' in answer.lower(), answer
print(f'PASS: local {backend.upper()} streaming inference: {answer}')
