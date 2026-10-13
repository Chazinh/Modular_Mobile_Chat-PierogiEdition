"""Download pinned GGUF artifacts with publisher SHA-256 verification; no activation/removal."""
import hashlib
import json
import pathlib
import urllib.request

dest = pathlib.Path(__file__).resolve().parents[1] / 'data' / 'model-staging'
dest.mkdir(parents=True, exist_ok=True)
models = [('unsloth/Qwen3.5-4B-GGUF', 'Qwen3.5-4B-Q5_K_M.gguf'),
          ('ggml-org/embeddinggemma-300M-GGUF', 'embeddinggemma-300M-Q8_0.gguf')]
manifest = []
for repo, name in models:
    with urllib.request.urlopen(f'https://huggingface.co/api/models/{repo}?blobs=true', timeout=40) as response:
        meta = json.load(response)
    file = next(f for f in meta['siblings'] if f['rfilename'].lower() == name.lower())
    name = file['rfilename']
    assert pathlib.Path(name).name == name
    sha = file['lfs']['sha256']
    target = dest / name
    url = f'https://huggingface.co/{repo}/resolve/{meta["sha"]}/{name}'
    if not target.exists():
        part = target.with_suffix('.part')
        digest = hashlib.sha256()
        print(f'Download: {name} ({file["size"]} bytes)', flush=True)
        with urllib.request.urlopen(url, timeout=60) as response, part.open('wb') as out:
            while block := response.read(4 * 1024 * 1024):
                digest.update(block)
                out.write(block)
        if digest.hexdigest() != sha or part.stat().st_size != file['size']:
            raise RuntimeError(f'Checksum mismatch: {name}')
        part.replace(target)
    with target.open('rb') as stream:
        assert hashlib.file_digest(stream, 'sha256').hexdigest() == sha
    manifest.append(dict(repo=repo, revision=meta['sha'], filename=name, sha256=sha, size=target.stat().st_size, url=url))
    print(f'Verified: {name}', flush=True)
(dest / 'manifest.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
