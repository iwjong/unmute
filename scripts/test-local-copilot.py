#!/usr/bin/env python3
"""Real on-device smoke test; no mocks, API keys or external speech services.
Pass the bundled app's extracted helper path or a compiled Swift helper.
"""
import argparse, json, pathlib, struct, subprocess, threading, time, wave, socket, os, signal
parser = argparse.ArgumentParser()
parser.add_argument('--helper', required=True)
parser.add_argument('--runtime', default='src-tauri/resources/local-runtime')
parser.add_argument('--models', default=str(pathlib.Path.home() / 'Library/Application Support/com.iwjong.unmute/models'))
parser.add_argument('--fixture', default='fixtures/stt/b-short-questions.wav')
parser.add_argument('--pause', action='store_true', help='Pause, request a response, then resume the same recognizer')
parser.add_argument('--jitter', action='store_true', help='Exercise sub-sample host timestamp overlap')
parser.add_argument('--output', default='reports/local-copilot-smoke.jsonl')
args = parser.parse_args()
dependencies = subprocess.check_output(['otool', '-L', args.helper], text=True)
assert 'FoundationModels.framework' not in dependencies, 'Apple Intelligence dependency must not return'
output = pathlib.Path(args.output)
output.parent.mkdir(parents=True, exist_ok=True)
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0)); port = listener.getsockname()[1]
process = subprocess.Popen([args.helper, args.models, str(port)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, start_new_session=True)
start = time.monotonic()
events, errors = [], []
server = None
def replay():
    try:
        with wave.open(args.fixture) as wav:
            assert (wav.getframerate(), wav.getnchannels(), wav.getsampwidth()) == (16000, 1, 2)
            epoch = time.monotonic()
            samples = 0
            paused = False
            gap = 0
            while data := wav.readframes(341 if args.jitter else 1280):
                if args.pause and not paused and samples >= 7 * 16000:
                    process.stdin.write(struct.pack('<qI', -1, 0)); process.stdin.flush()
                    time.sleep(8)
                    process.stdin.write(struct.pack('<qI', -2, 0)); process.stdin.flush()
                    time.sleep(3)
                    gap = 11
                    paused = True
                deadline = epoch + gap + (samples + len(data) // 2) / 16000
                time.sleep(max(0, deadline - time.monotonic()))
                timestamp = samples * 62500 + (10000 if samples % 2 else -10000) if args.jitter else samples * 62500
                process.stdin.write(struct.pack('<qI', timestamp + int(gap * 1e9), len(data)) + data)
                process.stdin.flush()
                samples += len(data) // 2
        process.stdin.close()
    except Exception as error:
        errors.append(str(error))
def kill_group():
    try: os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError: pass
timer = threading.Timer(180, kill_group)
timer.start()
try:
    with output.open('w') as report:
        for line in process.stdout:
            event = json.loads(line)
            event['received_elapsed_ms'] = (time.monotonic() - start) * 1000
            report.write(json.dumps(event) + '\n'); report.flush()
            events.append(event)
            if event.get('type') == 'model_ready':
                server = subprocess.Popen([str(pathlib.Path(args.runtime).resolve() / 'llama-server'), '-m', str(pathlib.Path(args.models) / 'Qwen3-1.7B-Q8_0.gguf'), '--host', '127.0.0.1', '--port', str(port), '-c', '4096', '-np', '1', '--reasoning', 'off', '--no-webui', '--no-warmup', '-ngl', '99'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if event.get('status') == 'Listening': threading.Thread(target=replay).start()
    assert process.wait() == 0, 'helper failed or timed out'
finally:
    timer.cancel()
    kill_group()
    if server is not None:
        server.kill(); server.wait()
if args.pause:
    pause = next(e for e in events if e.get('status') == 'Paused')
    after = [e for e in events if e['received_elapsed_ms'] > pause['received_elapsed_ms']]
    next_speech = next(e for e in after if e.get('type') == 'transcript')
    assert any(e.get('state') == 'READY' and e['received_elapsed_ms'] < next_speech['received_elapsed_ms'] for e in after), 'No response while paused'
    assert len({e['id'] for e in after if e.get('type') == 'transcript'}) >= 1
finals = [e for e in events if e.get('final')]
assert len(finals) >= (3 if 'b-short-questions' in args.fixture else 1), finals
assert any(e.get('final') is False for e in events)
ready = [e for e in events if e.get('state') == 'READY']
assert ready, 'No generated responses'
for event in ready:
    choices = event.get('options', [])
    assert [c['kind'] for c in choices] == ['question', 'agreement', 'idea', 'next_step'], event
    assert all(0 < len(c['text']) <= 180 for c in choices), event
    assert len({c['text'].lower() for c in choices}) == 4, event
assert not any(e.get('state') == 'ERROR' for e in events), 'Generation error; inspect raw evidence'
assert not errors and not any(e['type'] == 'error' for e in events), errors
for final in finals:
    assert any(e.get('id') == final['id'] and e.get('final') is False for e in events)
print('Real local speech + generation passed:', [e['text'] for e in finals])
