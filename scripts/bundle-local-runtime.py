#!/usr/bin/env python3
"""Bundle a checksum-pinned, portable official llama.cpp release. Stdlib only."""
import hashlib, os, pathlib, shutil, subprocess, tarfile, urllib.request
root = pathlib.Path(__file__).resolve().parents[1]
arch = 'arm64' if os.environ.get('CARGO_CFG_TARGET_ARCH', 'aarch64') == 'aarch64' else 'x64'
version = 'b11321'
expected = {
    'arm64': '5f47ffa4de936853004e7403a09d87616022e5af16651d71fc66a96b261886fb',
    'x64': '1e6e53f2a3dc3a412df69dd0f4495571ea1fece32397362eebc02ac89bc8a0ec',
}[arch]
cache = root / 'src-tauri/target/runtime-cache'
cache.mkdir(parents=True, exist_ok=True)
archive = cache / f'llama-{version}-bin-macos-{arch}.tar.gz'
if not archive.exists():
    temporary = archive.with_suffix('.download')
    urllib.request.urlretrieve(f'https://github.com/ggml-org/llama.cpp/releases/download/{version}/{archive.name}', temporary)
    assert hashlib.sha256(temporary.read_bytes()).hexdigest() == expected, 'Runtime checksum mismatch'
    temporary.rename(archive)
assert hashlib.sha256(archive.read_bytes()).hexdigest() == expected, 'Runtime checksum mismatch'
out = root / 'src-tauri/resources/local-runtime'
if out.exists(): shutil.rmtree(out)
out.mkdir(parents=True)
with tarfile.open(archive) as tar:
    for member in tar.getmembers():
        name = pathlib.PurePosixPath(member.name).name
        if name not in ('llama-server', 'LICENSE') and not name.endswith('.dylib'): continue
        target = out / name
        if member.issym():
            target.symlink_to(pathlib.PurePosixPath(member.linkname).name)
        elif member.isfile():
            with tar.extractfile(member) as source, target.open('wb') as destination:
                shutil.copyfileobj(source, destination)
            target.chmod(0o755 if name != 'LICENSE' else 0o644)
# Disallow developer-machine dependencies in the shipped runtime.
for path in out.iterdir():
    if path.name == 'llama-server' or path.suffix == '.dylib':
        dependencies = subprocess.check_output(['otool', '-L', str(path)], text=True)
        assert '/opt/homebrew/' not in dependencies and '/usr/local/' not in dependencies, dependencies
shutil.copy2(root / 'docs/licenses/Qwen3-LICENSE', out / 'Qwen3-LICENSE')
print(f'Bundled portable llama.cpp {version} ({arch})')
