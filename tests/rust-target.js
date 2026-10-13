// Where cargo puts the Rust executables. The build directory may live outside the project (CARGO_TARGET_DIR
// or `build.target-dir` in a .cargo/config.toml), e.g. to keep a cloud-sync client away from files the linker
// is writing; `cargo metadata` resolves it the same way cargo does.
import { execFileSync } from 'node:child_process';
import path from 'node:path';

let cached;
export function rustTargetDir() {
  if (cached) return cached;
  if (process.env.CARGO_TARGET_DIR) return (cached = path.resolve(process.env.CARGO_TARGET_DIR));
  try {
    const out = execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps', '--offline', '--manifest-path', path.resolve('server-rust/Cargo.toml')], { windowsHide: true, stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 16 * 1024 * 1024 });
    cached = JSON.parse(out).target_directory;
  } catch {
    cached = path.resolve('server-rust/target');
  }
  return cached;
}
export const rustExe = (name, profile = 'debug') => path.join(rustTargetDir(), profile, `${name}${process.platform === 'win32' ? '.exe' : ''}`);
