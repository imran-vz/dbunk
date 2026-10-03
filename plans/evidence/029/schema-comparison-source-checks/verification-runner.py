import json, os, pathlib, subprocess, sys, time
root = pathlib.Path('/Users/imran/projects/Code/dbunk')
out = root / 'plans/evidence/029/schema-comparison-source-checks'
mode = sys.argv[1]
if len(sys.argv) > 2:
 out = out / sys.argv[2]
 out.mkdir(exist_ok=False)
cargo_native = ['cargo', '+1.98.1']
native_manifest = ['--manifest-path', 'apps/native/Cargo.toml']
backend_manifest = ['--manifest-path', 'src-tauri/Cargo.toml']
matrices = {
 'native': [
  ('native-format', cargo_native + ['fmt'] + native_manifest + ['--check']),
  ('native-clippy', cargo_native + ['clippy'] + native_manifest + ['--locked', '--all-targets', '--', '-D', 'warnings']),
  ('native-harness-clippy', cargo_native + ['clippy'] + native_manifest + ['--locked', '--features', 'fixture-verification', '--all-targets', '--', '-D', 'warnings']),
  ('native-test', cargo_native + ['test'] + native_manifest + ['--locked']),
  ('native-build', cargo_native + ['build'] + native_manifest + ['--locked']),
  ('native-release-clippy', cargo_native + ['clippy'] + native_manifest + ['--release', '--locked', '--all-targets', '--', '-D', 'warnings']),
  ('native-release-test', cargo_native + ['test'] + native_manifest + ['--release', '--locked']),
  ('dependency-proof', ['python3', 'tools/native/dependencies.py']),
 ],
 'backend': [
  ('rust-fmt', ['just', 'fmt']),
  ('rust-lint', ['just', 'lint']),
  ('rust-test', ['just', 'test']),
  ('isolated-clippy', ['cargo', '+1.97.1', 'clippy'] + backend_manifest + ['--locked', '--no-default-features', '--features', 'isolated-profile', '--all-targets', '--', '-D', 'warnings']),
  ('isolated-test', ['cargo', '+1.97.1', 'test'] + backend_manifest + ['--locked', '--no-default-features', '--features', 'isolated-profile']),
  ('tauri-facade-test', ['cargo', '+1.97.1', 'test'] + backend_manifest + ['--locked', '--features', 'isolated-profile', 'backend::']),
  ('custom-protocol', ['cargo', '+1.97.1', 'build'] + backend_manifest + ['--locked', '--features', 'tauri/custom-protocol']),
 ],
}
env = os.environ.copy()
env['CARGO_INCREMENTAL'] = '0'
env['RUST_TEST_THREADS'] = '1'
results = []
if len(sys.argv) > 3:
 matrices[mode] = matrices[mode][int(sys.argv[3]):]
for name, command in matrices[mode]:
 print(f'{name}: starting', flush=True)
 start = time.monotonic()
 with (out / f'{name}.txt').open('x') as log:
  result = subprocess.run(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
 results.append({'name': name, 'command': command, 'exit': result.returncode, 'elapsed_seconds': round(time.monotonic()-start, 2), 'environment': {'CARGO_INCREMENTAL': '0', 'RUST_TEST_THREADS': '1'}})
 (out / f'{mode}-matrix.json').write_text(json.dumps(results, indent=2)+'\n')
 print(f'{name}: exit {result.returncode}', flush=True)
 if result.returncode:
  sys.exit(result.returncode)
